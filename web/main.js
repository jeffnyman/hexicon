// A page that plays a game with Hexicon.
//
// The game runs until it needs something from the player, then stops and
// says what: a line of input, a keystroke, or a pause. The page supplies
// it and tells the game to carry on.
//
// The page is a rich display: the game's markup, checked by Hexicon
// against its profile of HTML and CSS, is shown as the page's own, with
// its pictures, sounds, links, and panes, and its style sheets. The
// player's settings come with it, and can come first. A setting makes the
// page a display of plain text instead, as a terminal is.

import init, { Game } from "./pkg/hexicon_wasm.js";

const element = (id) => document.getElementById(id);
const gameArea = element("game");
const screen = element("screen");
const transcript = element("transcript");
const entry = element("entry");
const command = element("command");
const notice = element("notice");
const more = element("more");

let game = null;
let gameName = "";
// What the game is waiting for: "line", "key", "sleep", or "done".
let state = "done";
// Whether this game is shown as a rich display.
let rich = true;

// ---- the player's settings ----

const defaultSettings = {
  display: "rich", // or "text"
  theme: "game", // "light", "dark", "contrast"
  font: "game", // "serif", "sans", "mono"
  size: 100, // percent
  width: "game", // "narrow", "medium", "wide", "full"
  pictures: true,
  sounds: true,
  music: true,
  links: true,
  mineFirst: false,
  css: "",
};

function loadSettings() {
  try {
    return { ...defaultSettings, ...JSON.parse(localStorage.getItem("hexicon:settings") ?? "{}") };
  } catch {
    return { ...defaultSettings };
  }
}

let settings = loadSettings();

function saveSettings() {
  try {
    localStorage.setItem("hexicon:settings", JSON.stringify(settings));
  } catch {
    // A browser that keeps nothing still plays.
  }
}

// ---- the game's files ----
//
// In a browser they are kept in the browser's storage. In Hexicon's
// player, which gives the page window.ipc to send it messages, they are
// files in a folder of the player's own, one folder for each game: the
// page asks for them when it opens a game, and sends each one the game
// writes or deletes.

const inPlayer = typeof window.ipc?.postMessage === "function";

const storagePrefix = () => `hexicon:${gameName}:`;

const toBase64 = (bytes) => {
  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return btoa(text);
};

const fromBase64 = (text) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

// The files kept in the browser's storage for this game: their names and
// their keys.
function storedFiles() {
  const prefix = storagePrefix();
  const files = [];
  try {
    for (let index = 0; index < localStorage.length; index++) {
      const key = localStorage.key(index);
      if (key.startsWith(prefix)) {
        files.push([key.slice(prefix.length), key]);
      }
    }
  } catch {
    // A browser that keeps nothing has nothing kept.
  }
  return files;
}

// Give the game the files it wrote when it was last played here.
async function restoreFiles() {
  if (!inPlayer) {
    for (const [name, key] of storedFiles()) {
      game.putFile(name, fromBase64(localStorage.getItem(key)));
    }
    return;
  }
  const names = new Set();
  try {
    const listing = await (await fetch(`files/${encodeURIComponent(gameName)}`)).text();
    for (const line of listing.split("\n")) {
      const [name, contents] = line.split(" ");
      if (contents !== undefined) {
        names.add(decodeURIComponent(name));
        game.putFile(decodeURIComponent(name), fromBase64(contents));
      }
    }
  } catch (error) {
    say(`The saved games could not be read: ${error.message}`);
  }
  // The player kept a game's files in the web view's storage before it
  // kept them in a folder: they go to the folder, unless it has a file
  // of that name already.
  for (const [name, key] of storedFiles()) {
    if (!names.has(name)) {
      const contents = fromBase64(localStorage.getItem(key));
      game.putFile(name, contents);
      sendFile(name, contents);
    }
    localStorage.removeItem(key);
  }
}

// Tell the player of a file the game has written, or deleted.
function sendFile(name, contents) {
  const words = [encodeURIComponent(gameName), encodeURIComponent(name)];
  if (contents === undefined) {
    window.ipc.postMessage(`delete ${words.join(" ")}`);
  } else {
    window.ipc.postMessage(`write ${words.join(" ")} ${toBase64(contents)}`);
  }
}

// What the player says when it could not do what the page asked.
window.hexiconPlayer = {
  failed(name, message) {
    say(
      name
        ? `The file "${name}" could not be used: ${message}`
        : `The folder could not be shown: ${message}`,
    );
  },

  // The file the player chose in the dialog the game asked for, by the
  // name the game is to know it by, with its contents if it is to be
  // read; or null for none.
  fileChosen(name, contents) {
    if (state !== "file") {
      return;
    }
    if (name !== null && contents !== undefined) {
      game.putFile(name, fromBase64(contents));
    }
    game.sendFileChoice(name ?? undefined);
    advance();
  },
};

// In the player, the game's file dialogs are the system's: the player
// shows one, starting in the game's folder.
function askForFile() {
  const asked = JSON.parse(game.fileDialog());
  const words = [
    asked.save ? "save" : "open",
    encodeURIComponent(gameName),
    encodeURIComponent(asked.prompt),
    asked.kind,
  ];
  window.ipc.postMessage(`dialog ${words.join(" ")}`);
}

// Store the files the game has written, and forget the ones it deleted.
function storeFiles() {
  for (const name of game.takeChangedFiles()) {
    const contents = game.getFile(name);
    if (inPlayer) {
      sendFile(name, contents);
      continue;
    }
    try {
      if (contents === undefined) {
        localStorage.removeItem(storagePrefix() + name);
      } else {
        localStorage.setItem(storagePrefix() + name, toBase64(contents));
      }
    } catch {
      say(`The browser has no room to keep the file "${name}".`);
    }
  }
}

// ---- the game's resources ----

const resourceTypes = {
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  png: "image/png",
  svg: "image/svg+xml",
  webp: "image/webp",
  gif: "image/gif",
  wav: "audio/wav",
  ogg: "audio/ogg",
  mp3: "audio/mpeg",
  woff: "font/woff",
  woff2: "font/woff2",
  ttf: "font/ttf",
  otf: "font/otf",
};

const resourceUrls = new Map();

// An address for one of the game's resources, or null if it has none by
// that name.
function resourceUrl(name) {
  if (resourceUrls.has(name)) {
    return resourceUrls.get(name);
  }
  const bytes = game.resource(name);
  let url = null;
  if (bytes !== undefined) {
    const extension = name.split(".").pop().toLowerCase();
    const type = resourceTypes[extension] ?? "application/octet-stream";
    url = URL.createObjectURL(new Blob([bytes], { type }));
  }
  resourceUrls.set(name, url);
  return url;
}

function forgetResources() {
  for (const url of resourceUrls.values()) {
    if (url) {
      URL.revokeObjectURL(url);
    }
  }
  resourceUrls.clear();
}

// A style's url(...) of a resource, as the resource's address.
const withResourceUrls = (css) =>
  css.replace(/url\(\s*(['"]?)([^'")]+)\1\s*\)/g, (whole, quote, name) => {
    const url = resourceUrl(name.trim());
    return url ? `url("${url}")` : "none";
  });

// ---- styles: the page's defaults, the player's, and the game's ----

// The colors TADS games name, as the theme has them.
const namedColors = {
  text: "var(--hx-text)",
  bgcolor: "var(--hx-bg)",
  link: "var(--hx-link)",
  hlink: "var(--hx-hlink)",
  alink: "var(--hx-alink)",
  statustext: "var(--hx-status-text)",
  statusbg: "var(--hx-status-bg)",
  input: "var(--hx-input)",
};

const colorOf = (value) => namedColors[value.toLowerCase()] ?? value;

// The page's defaults come first, then the player's settings, then the
// game's style sheets, which win over the player's unless the player
// says theirs come first. The game's own style attributes win over all
// but that.
function applyStyles() {
  const fonts = {
    serif: 'Georgia, "Times New Roman", serif',
    sans: "system-ui, sans-serif",
    mono: 'ui-monospace, "Cascadia Mono", Consolas, monospace',
  };
  const widths = { narrow: "36em", medium: "48em", wide: "64em", full: "none" };
  const themes = {
    light: ["#fbfaf6", "#1f1d1a", "#7a4b12", "#e8e2d2", "#1f1d1a"],
    dark: ["#16161a", "#e7e3da", "#e0a85a", "#2b2a30", "#e7e3da"],
    contrast: ["#000000", "#ffffff", "#ffff00", "#ffffff", "#000000"],
  };
  const important = settings.mineFirst ? " !important" : "";
  const rule = (selector, declarations) =>
    declarations.length
      ? `${selector} { ${declarations.map((d) => d + important).join("; ")} }\n`
      : "";

  let player = "";
  const theme = themes[settings.theme];
  if (theme) {
    const [bg, text, link, statusBg, statusText] = theme;
    player += rule("#game", [
      `--hx-bg: ${bg}`,
      `--hx-text: ${text}`,
      `--hx-link: ${link}`,
      `--hx-hlink: ${link}`,
      `--hx-alink: ${link}`,
      `--hx-input: ${link}`,
      `--hx-status-bg: ${statusBg}`,
      `--hx-status-text: ${statusText}`,
    ]);
    if (settings.mineFirst) {
      player += rule("#game #screen, #game .rich-pane", [
        `background: var(--hx-bg)`,
        `color: var(--hx-text)`,
      ]);
      player += rule("#game #screen *, #game .rich-pane *", [
        `color: inherit`,
        `background-color: transparent`,
      ]);
      player += rule("#game #screen a[data-command], #game #screen .hx-input, #game #screen .inputline, #game #screen kbd", [
        `color: var(--hx-link)`,
      ]);
    }
  }
  if (fonts[settings.font]) {
    player += rule("#game", [`font-family: ${fonts[settings.font]}`]);
    if (settings.mineFirst) {
      player += rule("#game *:not(kbd):not(.hx-input)", [`font-family: inherit`]);
    }
  }
  if (settings.size !== 100) {
    player += rule("#game", [`font-size: ${settings.size}%`]);
  }
  if (widths[settings.width]) {
    player += rule("#game #transcript", [`max-width: ${widths[settings.width]}`]);
  }
  if (!settings.pictures) {
    player += "#game img { display: none !important }\n";
  }
  if (!settings.links) {
    player +=
      "#game [data-command] { pointer-events: none !important; color: inherit !important; text-decoration: none !important }\n";
  }
  player += settings.css;

  const gameSheets = game && rich ? withResourceUrls(game.styleSheets("#game")) : "";
  element("styles").textContent =
    "@layer base, player, game;\n" +
    `@layer player {\n${player}\n}\n` +
    `@layer game {\n${gameSheets}\n}\n`;
}

// ---- sounds ----

// Sounds play in layers, as in HTML TADS: a place's background and
// ambient sounds, music in the background, and the foreground's sounds
// that things make. A sound in one layer does not stop one in another.
//
// A sound can have a volume (0 to 100), and fade in when it starts and
// out when it ends (fadein and fadeout, in seconds). A sound with a
// fadeout fades out whenever it stops, cancelled or interrupted as well
// as at its end; a cancel with a fadeout of its own fades out everything
// it stops.
//
// They play through the Web Audio API, the same way on every system: each
// of the game's sounds is decoded once, when the game starts, and the page
// says when its samples play. So a sound starts at once, loops without a
// break, and fades on the audio's own clock, where a media element for
// each, which the system's own player plays, started late on Linux and
// left a gap where it looped. The browser lets a page play sound once the
// player has done something on it; until then the audio waits, and what
// was started plays then.
const sounds = {
  // The page's audio, made when it is first needed.
  context: null,
  // Each sound decoded, by its name: a promise of its audio, or of null
  // for one that cannot be.
  buffers: new Map(),
  // The sounds playing in each layer.
  layers: new Map(),
  // Which game the sounds are for: a sound decoded for one game is not
  // played in the next.
  generation: 0,

  audio() {
    if (!this.context) {
      const Context = window.AudioContext ?? window.webkitAudioContext;
      if (!Context) {
        return null;
      }
      this.context = new Context();
    }
    return this.context;
  },

  // A sound, decoded, once.
  buffer(name) {
    if (!this.buffers.has(name)) {
      const context = this.audio();
      const bytes = game?.resource(name);
      this.buffers.set(
        name,
        !context || bytes === undefined
          ? Promise.resolve(null)
          : context.decodeAudioData(bytes.buffer).catch(() => null),
      );
    }
    return this.buffers.get(name);
  },

  // A new game: no sounds playing, and its own decoded.
  reset(names) {
    this.stop(null);
    this.buffers.clear();
    this.generation += 1;
    for (const name of names) {
      if (/\.(wav|ogg|mp3)$/i.test(name)) {
        this.buffer(name);
      }
    }
  },

  layer(name) {
    if (!this.layers.has(name)) {
      this.layers.set(name, new Set());
    }
    return this.layers.get(name);
  },

  // The player has done something: the audio can play.
  resume() {
    if (this.context?.state === "suspended") {
      this.context.resume().catch(() => {});
    }
  },

  // Stop the sounds of a layer, or of every layer. With fades, each fades
  // out first, over the time given, or its own fadeout.
  stop(name, fades = false, seconds = NaN) {
    const names = name ? [name] : [...this.layers.keys()];
    for (const layerName of names) {
      for (const entry of this.layer(layerName)) {
        entry.stopped = true;
        clearInterval(entry.timer);
        const fade = Number.isNaN(seconds) ? entry.fadeOut : seconds;
        const source = entry.source;
        if (!source) {
          entry.gain.disconnect();
          continue;
        }
        const now = this.context.currentTime;
        try {
          if (fades && fade > 0) {
            const gain = entry.gain.gain;
            gain.cancelScheduledValues(now);
            gain.setValueAtTime(gain.value, now);
            gain.linearRampToValueAtTime(0, now + fade);
            source.stop(now + fade);
          } else {
            source.stop();
          }
        } catch {
          // It had stopped already.
        }
      }
      this.layer(layerName).clear();
    }
  },

  // Play a sound some number of times (Infinity for over and over) as one
  // run: looped in the audio, so there is no break between the times.
  play(entry, buffer, times) {
    const context = this.context;
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.loop = times > 1;
    source.connect(entry.gain);
    entry.source = source;
    const now = context.currentTime;
    const gain = entry.gain.gain;
    gain.cancelScheduledValues(now);
    // It fades in the first time it plays, not each time it repeats.
    let full = now;
    if (entry.fadeIn > 0 && !entry.played) {
      gain.setValueAtTime(0, now);
      gain.linearRampToValueAtTime(entry.volume, now + entry.fadeIn);
      full = now + entry.fadeIn;
    } else {
      gain.setValueAtTime(entry.volume, now);
    }
    entry.played = true;
    if (Number.isFinite(times)) {
      const end = now + times * buffer.duration;
      if (times > 1) {
        source.stop(end);
      }
      // The last time it plays, it fades out before its end.
      if (entry.fadeOut > 0 && entry.left <= times) {
        gain.setValueAtTime(entry.volume, Math.max(full, end - entry.fadeOut));
        gain.linearRampToValueAtTime(0, end);
      }
    }
    source.onended = () => {
      if (entry.source === source) {
        entry.source = null;
      }
      // One that plays now and then waits for its next time.
      if (entry.chance === null || entry.stopped || entry.left <= 0) {
        this.layer(entry.layer).delete(entry);
        if (!entry.source) {
          entry.gain.disconnect();
        }
      }
    };
    source.start(now);
  },

  // A <sound> tag.
  handle(attrs) {
    const seconds = (value) => {
      const number = parseFloat(value);
      return Number.isFinite(number) && number > 0 ? Math.min(number, 60) : 0;
    };
    if ("cancel" in attrs) {
      const fade = attrs.fadeout === undefined ? NaN : seconds(attrs.fadeout);
      this.stop(attrs.cancel ? attrs.cancel.toLowerCase() : null, true, fade);
      return;
    }
    const layer = (attrs.layer ?? "foreground").toLowerCase();
    const music = layer === "background";
    if (!settings.sounds || (music && !settings.music) || !attrs.src) {
      return;
    }
    const context = this.audio();
    if (!context) {
      return;
    }
    if ("interrupt" in attrs) {
      this.stop(layer, true);
    }
    const volume = parseFloat(attrs.volume);
    const repeat = (attrs.repeat ?? "1").toLowerCase();
    const chance = attrs.random !== undefined ? parseFloat(attrs.random) : NaN;
    const entry = {
      name: attrs.src,
      layer,
      gain: context.createGain(),
      source: null,
      timer: null,
      stopped: false,
      played: false,
      volume: Number.isFinite(volume) ? Math.min(Math.max(volume, 0), 100) / 100 : 1,
      fadeIn: seconds(attrs.fadein),
      fadeOut: seconds(attrs.fadeout),
      left: repeat === "loop" ? Infinity : Math.max(1, parseInt(repeat, 10) || 1),
      // Now and then: each second, this chance in a hundred that it
      // plays, if it is not playing already.
      chance: Number.isNaN(chance) ? null : chance,
    };
    entry.gain.connect(context.destination);
    this.layer(layer).add(entry);
    const generation = this.generation;
    this.buffer(attrs.src).then((buffer) => {
      if (!buffer || generation !== this.generation || entry.stopped) {
        this.layer(layer).delete(entry);
        entry.gain.disconnect();
        return;
      }
      if (entry.chance === null) {
        this.play(entry, buffer, entry.left);
        entry.left = 0;
        return;
      }
      entry.timer = setInterval(() => {
        if (!entry.source && entry.left > 0 && Math.random() * 100 < entry.chance) {
          this.play(entry, buffer, 1);
          entry.left -= 1;
        }
      }, 1000);
    });
  },
};

// What is playing, for checks: each sound's layer, name, and whether it
// is sounding now, waiting for its next time, or for the audio to start.
window.hexiconSoundsPlaying = () =>
  [...sounds.layers.entries()].flatMap(([layer, entries]) =>
    [...entries].map((entry) => ({
      layer,
      name: entry.name,
      sounding: entry.source !== null && sounds.context?.state === "running",
      looping: entry.source?.loop ?? false,
      volume: Math.round(entry.gain.gain.value * 100) / 100,
      audio: sounds.context?.state ?? "none",
    })),
  );

// ---- the rich display: the document the game makes ----

// Reads the game's checked items into the elements of one window: the
// main window, or a pane. Elements are opened and closed as the items
// say; the checker has put them in order.
class Builder {
  constructor(root) {
    this.root = root;
    this.stack = [{ name: "#root", node: root }];
    // A <tab align=right> or <tab align=center>: what follows, to the next
    // tab or the end of the line, goes to the right or the middle.
    this.rightOf = null;
  }

  get top() {
    return this.stack[this.stack.length - 1].node;
  }

  reset() {
    this.root.replaceChildren();
    this.stack = [{ name: "#root", node: this.root }];
  }

  append(node) {
    this.top.append(node);
    if (!this.inBanner()) {
      unseen ??= node;
    }
  }

  // Whether what is added goes to a <banner>, and not the window.
  inBanner() {
    return this.stack.some((entry) => entry.banner);
  }

  text(text) {
    this.append(document.createTextNode(text));
  }

  open(name, attrs) {
    if (name === "aboutbox") {
      const box = document.createElement("div");
      this.stack.push({ name, node: box, about: true });
      return;
    }
    if (name === "banner") {
      // What is in it replaces what the banner of that id had.
      this.stack.push({ name, node: tagBanners.open(attrs), banner: true });
      return;
    }
    const node = makeElement(name, attrs);
    this.append(node);
    this.stack.push({ name, node });
  }

  close(name) {
    // The checker closes what it opened, in order; anything else is let
    // be.
    if (!this.stack.some((entry, index) => index > 0 && entry.name === name)) {
      return;
    }
    while (this.stack.length > 1) {
      const top = this.stack.pop();
      if (top.node === this.rightOf) {
        this.rightOf = null;
      }
      if (top.about) {
        showAboutBox(top.node);
      }
      if (top.name === name) {
        return;
      }
    }
  }

  void(name, attrs) {
    switch (name) {
      case "sound":
        sounds.handle(attrs);
        return;
      case "body":
        this.body(attrs);
        return;
      case "tab": {
        // What follows, to the next tab or the end of the line, goes to
        // the right, or in the middle.
        const align = (attrs.align ?? "").toLowerCase();
        if (align === "right" || align === "center") {
          this.endRight();
          const span = document.createElement("span");
          span.className = `hx-${align}`;
          this.append(span);
          this.stack.push({ name: "#right", node: span });
          this.rightOf = span;
        }
        return;
      }
      case "basefont":
        return;
      case "banner":
        tagBanners.remove(attrs);
        return;
      case "br":
        this.endRight();
        break;
      default:
        break;
    }
    this.append(makeElement(name, attrs));
  }

  // The end of the line, or another tab, ends what a <tab> put at the
  // right or in the middle.
  endRight() {
    if (!this.rightOf) {
      return;
    }
    const at = this.stack.findIndex((entry) => entry.node === this.rightOf);
    if (at > 0) {
      this.stack.length = at;
    }
    this.rightOf = null;
  }

  // <body>: the window's colors, and its picture; in the about box, the
  // about box's. As in HTML TADS, each <body> says them all: one it
  // leaves out goes back to the window's own, so a pane's next picture
  // does not keep the last one's background.
  body(attrs) {
    const holder = this.stack.findLast((entry) => entry.about || entry.banner);
    const target = holder ? holder.node : this.root === transcript ? screen : this.root;
    target.style.backgroundColor = attrs.bgcolor ? colorOf(attrs.bgcolor) : "";
    target.style.color = attrs.text ? colorOf(attrs.text) : "";
    if (attrs.link) {
      target.style.setProperty("--hx-link", colorOf(attrs.link));
    } else {
      target.style.removeProperty("--hx-link");
    }
    const url = attrs.background ? resourceUrl(attrs.background) : "";
    target.style.backgroundImage = url ? `url("${url}")` : "";
  }

  // An item from the game.
  add(item) {
    switch (item[0]) {
      case "t":
        this.text(item[1]);
        break;
      case "o":
        this.open(item[1], attrsOf(item[2]));
        break;
      case "c":
        this.close(item[1]);
        break;
      case "v":
        this.void(item[1], attrsOf(item[2]));
        break;
      case "title":
        document.title = item[1] || "Hexicon";
        break;
      case "in": {
        const typed = document.createElement("kbd");
        typed.className = "typed";
        typed.textContent = item[1];
        this.append(typed);
        this.append(document.createElement("br"));
        break;
      }
      default:
        break;
    }
  }
}

const attrsOf = (list) => Object.fromEntries(list.map(([name, value]) => [name, value ?? ""]));

// Image maps are named, and a game may show the same one many times: each
// is given a name of its own, and a picture uses the one shown last.
const mapNames = new Map();
let mapCount = 0;

// One of the profile's elements, with its attributes as the page needs
// them: a link's or an area's command, a picture from the game's
// resources, colors the game names.
function makeElement(name, attrs) {
  const node = document.createElement(name);
  for (const [attr, value] of Object.entries(attrs)) {
    switch (attr) {
      case "href":
        node.dataset.command = value;
        node.setAttribute("href", "#");
        break;
      case "plain":
        node.classList.add("plain");
        break;
      case "append":
        node.dataset.append = "";
        break;
      case "noenter":
        node.dataset.noenter = "";
        break;
      case "src":
        if (name === "img") {
          const url = resourceUrl(value);
          if (url) {
            node.src = url;
          } else {
            node.dataset.missing = value;
          }
        }
        break;
      // The picture shown while the pointer is over it, and while it is
      // pressed.
      case "hsrc":
      case "asrc":
        if (name === "img") {
          const url = resourceUrl(value);
          if (url) {
            node.dataset[attr === "hsrc" ? "hover" : "pressed"] = url;
          }
        }
        break;
      case "background": {
        const url = resourceUrl(value);
        if (url) {
          node.style.backgroundImage = `url("${url}")`;
        }
        break;
      }
      case "style":
        node.setAttribute("style", withResourceUrls(value));
        break;
      case "name":
        if (name === "map") {
          const unique = `${value}-${++mapCount}`;
          mapNames.set(value, unique);
          node.name = unique;
        }
        break;
      case "usemap": {
        const named = value.replace(/^#/, "");
        node.useMap = `#${mapNames.get(named) ?? named}`;
        break;
      }
      case "face":
        if (value.toLowerCase() === "hexicon-input") {
          node.classList.add("hx-input");
        } else {
          node.setAttribute("face", value);
        }
        break;
      case "color":
      case "bgcolor":
      case "text":
        if (namedColors[value.toLowerCase()]) {
          node.style[attr === "bgcolor" ? "backgroundColor" : "color"] = colorOf(value);
        } else {
          node.setAttribute(attr, value);
        }
        break;
      default:
        node.setAttribute(attr, value);
        break;
    }
  }
  if (name === "img" && node.dataset.missing === undefined) {
    showsOnHover(node);
  }
  if (name === "img" && node.dataset.missing !== undefined) {
    // A picture the game does not have: its words, if any.
    const words = document.createElement("span");
    words.textContent = attrs.alt ?? "";
    return words;
  }
  return node;
}

// A picture with others for when the pointer is over it (hsrc) and when
// it is pressed (asrc), as on a button: a link's, mostly. Focus from the
// keyboard shows the hover picture too.
function showsOnHover(img) {
  const { hover, pressed } = img.dataset;
  if (!hover && !pressed) {
    return;
  }
  const normal = img.src;
  const over = hover ?? normal;
  const down = pressed ?? over;
  const show = (url) => () => {
    img.src = url;
  };
  img.addEventListener("pointerenter", show(over));
  img.addEventListener("pointerleave", show(normal));
  img.addEventListener("pointerdown", show(down));
  img.addEventListener("pointerup", show(over));
  // It is put in its link once it is made.
  queueMicrotask(() => {
    const link = img.closest("a");
    if (link) {
      link.addEventListener("focus", show(over));
      link.addEventListener("blur", show(normal));
    }
  });
}

const mainBuilder = new Builder(transcript);
// Each text pane's window, by the pane's id.
const richPanes = new Map();

function showAboutBox(box) {
  const content = element("about-content");
  // Its colors and picture, from a <body> in it.
  content.setAttribute("style", box.getAttribute("style") ?? "");
  content.replaceChildren(...box.childNodes);
  element("about").hidden = false;
}

function showDocument() {
  for (const event of JSON.parse(game.takeDocument())) {
    switch (event[0]) {
      case "pane-create": {
        const box = document.createElement("div");
        box.className = "rich-pane";
        box.dataset.pane = event[1];
        richPanes.set(event[1], new Builder(box));
        break;
      }
      case "pane-delete":
        richPanes.get(event[1])?.root.remove();
        richPanes.delete(event[1]);
        break;
      case "pane-clear":
        richPanes.get(event[1])?.reset();
        break;
      case "pane": {
        const builder = richPanes.get(event[1]);
        if (builder) {
          for (const item of event[2]) {
            builder.add(item);
          }
        }
        break;
      }
      default:
        mainBuilder.add(event);
        break;
    }
  }
}

// ---- the screen ----

// The most characters a line of the game's text has, however wide the
// window: longer lines are hard to read.
const widestLine = 80;
let columns = widestLine;

// How many characters fit across the screen, in the game's font.
function measureColumns() {
  const probe = document.createElement("span");
  probe.id = "measure";
  probe.textContent = "0".repeat(100);
  screen.append(probe);
  const characterWidth = probe.getBoundingClientRect().width / 100;
  probe.remove();
  const style = getComputedStyle(screen);
  const room =
    screen.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  if (!(characterWidth > 0) || !(room > 0)) {
    return widestLine;
  }
  return Math.max(20, Math.min(widestLine, Math.floor(room / characterWidth)));
}

// How many lines of text fit on the page below its header. The game's
// panes and its main window share them.
let rows = 24;

function measureRows() {
  const style = getComputedStyle(screen);
  const room =
    window.innerHeight -
    document.querySelector("header").offsetHeight -
    parseFloat(style.paddingTop) -
    parseFloat(style.paddingBottom);
  return Math.max(4, Math.floor(room / lineHeight()));
}

// Tell the game how much room there is. It takes effect for text shown
// after the game next waits for input; text already shown stays as it is.
function fitToWindow() {
  columns = measureColumns();
  rows = measureRows();
  if (game) {
    game.setWidth(columns);
    game.setHeight(rows);
    showPanes();
  }
  sizeCommandLine();
}

// ---- panes ----

const panesTop = element("panes-top");
const panesBottom = element("panes-bottom");

// Show the game's panes. On a display of plain text, those above its main
// window go in a strip above the text, and those below in a strip below
// it, laid out in lines and characters. A rich display lays them all out
// in its own measures.
function showPanes() {
  const layout = JSON.parse(game.panes());
  if (rich) {
    layOutPanes(layout.panes);
    return;
  }
  const mainTop = layout.main.y;
  const mainBottom = mainTop + layout.main.height;
  const above = layout.panes.filter((pane) => pane.y < mainTop);
  const below = layout.panes.filter((pane) => pane.y >= mainBottom);
  fillStrip(panesTop, above, 0, mainTop);
  fillStrip(panesBottom, below, mainBottom, rows - mainBottom);
}

// ---- panes on a rich display ----
//
// Each pane takes its side of what its parent has left, in the order the
// game made them, as in a terminal: a pane at the top or bottom the full
// width of what is left, and one at the left or right its full height.
// The main window, or a pane's own text, has what is left after its panes.
// A pane at the top or bottom is as tall as what is in it; one at a side
// is as wide as the game asks, in characters or as a percentage, or as
// wide as what is in it.
//
// A <banner> in the game's text is a pane too, after the others: what is
// in it replaces what the banner of that id had.

const ALIGN = ["top", "bottom", "left", "right"];
const PERCENT = 1;

// The panes made by <banner>, by id, in the order they were made.
const tagBanners = {
  banners: new Map(),

  open(attrs) {
    const id = attrs.id ?? "";
    let banner = this.banners.get(id);
    if (!banner) {
      const box = document.createElement("div");
      box.className = "rich-pane tag-banner";
      banner = { box, align: "top", size: "" };
      this.banners.set(id, banner);
    }
    banner.box.replaceChildren();
    banner.box.removeAttribute("style");
    const align = (attrs.align ?? "top").toLowerCase();
    banner.align = ALIGN.includes(align) ? align : "top";
    const across = banner.align === "left" || banner.align === "right";
    const size = ((across ? attrs.width : attrs.height) ?? "").trim().toLowerCase();
    if (size !== "previous") {
      banner.size = /^\d+(\.\d+)?%$/.test(size) ? size : /^\d+$/.test(size) ? `${size}px` : "";
    }
    banner.box.className = `rich-pane tag-banner ${attrs.class ?? ""}`.trim();
    banner.box.classList.toggle("border", "border" in attrs);
    if (attrs.style) {
      banner.box.setAttribute("style", withResourceUrls(attrs.style));
    }
    return banner.box;
  },

  remove(attrs) {
    if ("removeall" in attrs) {
      this.banners.clear();
    } else if ("remove" in attrs) {
      this.banners.delete(attrs.id ?? "");
    }
  },

  clear() {
    this.banners.clear();
  },
};

// Each text grid's box, by the pane's id, kept from one turn to the next.
const gridBoxes = new Map();
// What the layout was when it was last built.
let laidOut = "";

function layOutPanes(panes) {
  // What each pane is, and where it goes.
  const placed = [];
  for (const pane of panes) {
    let box = richPanes.get(pane.id)?.root;
    if (!box) {
      box = gridBoxes.get(pane.id);
      if (!box) {
        box = document.createElement("div");
        gridBoxes.set(pane.id, box);
      }
      box.className = pane.status ? "grid-pane status" : "grid-pane";
      box.textContent = pane.lines.join("\n");
    }
    const across = pane.align === 2 || pane.align === 3;
    let size = "";
    if (across && !pane.fitted) {
      size = pane.units === PERCENT ? `${pane.size}%` : `${pane.size}ch`;
    }
    // A pane with no room, or sized to nothing, is not shown.
    const shows = pane.fitted ? box.hasChildNodes() : pane.width > 0 && pane.height > 0;
    if (shows) {
      placed.push({
        key: `pane ${pane.id}`,
        parent: pane.parent === null ? null : `pane ${pane.parent}`,
        align: ALIGN[pane.align] ?? "top",
        size,
        box,
      });
    }
  }
  for (const [id, banner] of tagBanners.banners) {
    if (banner.box.hasChildNodes()) {
      placed.push({ key: `banner ${id}`, parent: null, ...banner });
    }
  }

  const shape = JSON.stringify(placed.map(({ box, ...rest }) => rest));
  if (shape === laidOut && placed.every((pane) => pane.box.isConnected)) {
    return;
  }
  laidOut = shape;

  const children = (key) => placed.filter((pane) => pane.parent === key);
  // Lay out a window's panes in a container, the window's own box in what
  // is left.
  const build = (container, panes, inner) => {
    let rest = container;
    for (const pane of panes) {
      const split = document.createElement("div");
      const across = pane.align === "left" || pane.align === "right";
      split.className = across ? "hx-split hx-across" : "hx-split";
      const next = document.createElement("div");
      next.className = "hx-rest";
      // The pane's place, and in it the pane, after its own panes.
      const holder = document.createElement("div");
      holder.className = `hx-pane hx-pane-${pane.align}`;
      holder.style.flexBasis = pane.size || "auto";
      build(holder, children(pane.key), pane.box);
      if (pane.align === "top" || pane.align === "left") {
        split.append(holder, next);
      } else {
        split.append(next, holder);
      }
      rest.append(split);
      rest = next;
    }
    rest.append(inner);
  };

  // Moving the main window's text keeps where it was scrolled to, and
  // the command line its focus.
  const scrolled = screen.scrollTop;
  const focused = document.activeElement === command;
  gameArea.replaceChildren();
  build(gameArea, children(null), screen);
  screen.scrollTop = scrolled;
  if (focused) {
    command.focus({ preventScroll: true });
  }
}

// Put the main window back as a display of plain text has it.
function resetPaneLayout() {
  laidOut = "";
  gridBoxes.clear();
  tagBanners.clear();
  gameArea.replaceChildren(panesTop, screen, panesBottom);
}

function fillStrip(strip, panes, firstRow, height) {
  const visible = panes.filter((pane) => pane.width > 0 && pane.height > 0);
  strip.hidden = height <= 0 || visible.length === 0;
  strip.style.height = `${height * lineHeight()}px`;
  strip.replaceChildren(
    ...visible.map((pane) => {
      const box = document.createElement("div");
      box.className = pane.status ? "pane status" : "pane";
      box.textContent = pane.lines.join("\n");
      box.style.top = `${(pane.y - firstRow) * lineHeight()}px`;
      box.style.height = `${pane.height * lineHeight()}px`;
      // The text lines up with the main window's, and a pane at an edge
      // of the screen reaches the edge of the page.
      if (pane.x === 0) {
        box.style.left = "0";
        box.style.paddingLeft = getComputedStyle(screen).paddingLeft;
      } else {
        box.style.left = `calc(${getComputedStyle(screen).paddingLeft} + ${pane.x}ch)`;
      }
      if (pane.x + pane.width >= columns + 1) {
        box.style.right = "0";
      } else {
        box.style.width = `${pane.width}ch`;
      }
      return box;
    }),
  );
}

// The command line takes up the rest of the line its prompt is on.
function sizeCommandLine() {
  if (rich) {
    fitCommandBesideFloats();
    return;
  }
  const text = transcript.textContent;
  const prompt = text.length - (text.lastIndexOf("\n") + 1);
  command.style.width = `${Math.max(columns - prompt - 1, 8)}ch`;
}

// On a rich display, the command line is as wide as its style says, but no
// wider than the room left on its line: beside a picture floated at the
// right, a line too wide for the room would be put below the picture,
// away from its prompt.
function fitCommandBesideFloats() {
  command.style.width = "";
  if (entry.hidden || !entry.isConnected) {
    return;
  }
  const natural = command.offsetWidth;
  // Where it starts when it is small enough to stay beside the prompt.
  command.style.width = "2em";
  const box = command.getBoundingClientRect();
  let right = transcript.getBoundingClientRect().right;
  // Only the last of them can be beside the last line.
  const candidates = [...transcript.querySelectorAll("img, figure, div, table")].slice(-40);
  for (const element of candidates) {
    const style = getComputedStyle(element);
    if (style.float === "none") {
      continue;
    }
    // Text keeps clear of the picture's margin too (an <img hspace>).
    const area = element.getBoundingClientRect();
    const left = area.left - parseFloat(style.marginLeft || "0");
    if (area.top < box.bottom && area.bottom > box.top && left >= box.left) {
      right = Math.min(right, left);
    }
  }
  const room = Math.floor(right - box.left - 8);
  command.style.width = room < natural ? `${Math.max(room, 32)}px` : "";
}

// A picture has its size once it has loaded, which may be after the
// command line was fitted beside it.
transcript.addEventListener(
  "load",
  (event) => {
    if (event.target.tagName === "IMG") {
      sizeCommandLine();
    }
  },
  true,
);

// Whether there is text below the bottom of the screen.
const lineHeight = () => parseFloat(getComputedStyle(screen).lineHeight) || 24;
const textBelow = () =>
  screen.scrollHeight - screen.scrollTop - screen.clientHeight > lineHeight() / 2;

function pageDown() {
  screen.scrollBy({ top: screen.clientHeight - 2 * lineHeight() });
}

function showMore() {
  more.hidden = !textBelow();
}

// Where the text the player has not yet seen starts, for scrolling to.
let unseen = null;

function say(text) {
  notice.textContent = text;
  notice.hidden = false;
}

// Plain text, as a terminal shows it.
function show(text, className) {
  if (text === "") {
    return;
  }
  const span = document.createElement("span");
  span.textContent = text;
  if (className) {
    span.className = className;
  }
  transcript.append(span);
  unseen ??= span;
}

// Bring new text into view. When there is more of it than fits, show it
// from its start and leave the player to read down to the end, instead
// of letting the start scroll away unread.
function scrollToNewText() {
  const bottom = screen.scrollHeight - screen.clientHeight;
  let top = bottom;
  const start = unseen?.nodeType === Node.TEXT_NODE ? unseen.parentNode : unseen;
  if (start && start.isConnected && start.getBoundingClientRect) {
    const at = start.getBoundingClientRect().top - screen.getBoundingClientRect().top + screen.scrollTop;
    top = Math.min(bottom, Math.max(0, at - lineHeight() / 2));
  }
  screen.scrollTop = top;
  unseen = null;
  showMore();
}

// Show what the game has displayed, and get ready for what it needs next.
function update() {
  if (game.takeCleared()) {
    mainBuilder.reset();
    unseen = null;
  }
  if (rich) {
    showDocument();
  } else {
    show(game.takeOutput());
  }
  storeFiles();
  showPanes();

  const left = game.status();
  const right = game.statusRight();
  element("status").hidden = left === "" && right === "";
  element("status-left").textContent = left;
  element("status-right").textContent = right;

  entry.hidden = state !== "line";
  if (state === "line" && rich) {
    // The command line goes where the game's text has got to: after its
    // prompt.
    mainBuilder.top.append(entry);
  }
  sizeCommandLine();
  if (state === "line") {
    command.focus({ preventScroll: true });
  } else if (state === "file") {
    askForFile();
  } else if (state === "sleep") {
    setTimeout(() => {
      game.wake();
      advance();
    }, game.sleepMillis());
  } else if (state === "done") {
    sounds.stop(null);
    say("The game has ended. Choose a game to play again.");
  }
  scrollToNewText();
}

function advance() {
  state = game.resume();
  update();
}

// ---- the player ----

// Give the game a line, as the player typed it.
function enterLine(line) {
  // The game does not display what the player types; a terminal would.
  if (rich) {
    entry.remove();
    mainBuilder.add(["in", line]);
  } else {
    show(line + "\n", "typed");
  }
  game.sendLine(line);
  advance();
}

entry.addEventListener("submit", (event) => {
  event.preventDefault();
  if (state !== "line") {
    return;
  }
  const line = command.value;
  command.value = "";
  enterLine(line);
});

// A link, or an area of a picture, is a command: entered, added to what
// is being typed, or put on the command line to be finished.
gameArea.addEventListener("click", (event) => {
  const link = event.target.closest("[data-command]");
  if (!link) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  if (state !== "line" || !settings.links) {
    return;
  }
  const words = link.dataset.command;
  if (link.dataset.append !== undefined) {
    command.value += words;
    command.focus();
  } else if (link.dataset.noenter !== undefined) {
    command.value = words;
    command.focus();
  } else {
    command.value = "";
    enterLine(words);
  }
});

// The names the game knows keys by, for the ones that type no character.
const keyNames = {
  Enter: "\n",
  Tab: "\t",
  Escape: "[esc]",
  Backspace: "[bksp]",
  Delete: "[del]",
  ArrowUp: "[up]",
  ArrowDown: "[down]",
  ArrowLeft: "[left]",
  ArrowRight: "[right]",
  Home: "[home]",
  End: "[end]",
  PageUp: "[page up]",
  PageDown: "[page down]",
  Insert: "[insert]",
};

// While there is text below the screen, the keys that would page through
// it in a terminal do that here: Page Down always, and the space bar when
// it is not being typed into a command.
command.addEventListener("keydown", (event) => {
  if (!textBelow()) {
    return;
  }
  if (event.key === "PageDown" || (event.key === " " && command.value === "")) {
    event.preventDefault();
    pageDown();
  }
});

screen.addEventListener("scroll", showMore);
more.addEventListener("click", pageDown);
window.addEventListener("resize", () => {
  fitToWindow();
  showMore();
});

// Sounds the browser held back until the player did something.
for (const kind of ["keydown", "pointerdown"]) {
  document.addEventListener(kind, () => sounds.resume(), { capture: true });
}

document.addEventListener("keydown", (event) => {
  if (state !== "key" || event.ctrlKey || event.metaKey || event.altKey) {
    return;
  }
  if (event.target.closest?.("dialog")) {
    return;
  }
  // A key pressed with text still to read shows more of it, as it would
  // at a More prompt, and does not reach the game.
  if (textBelow()) {
    event.preventDefault();
    pageDown();
    return;
  }
  let key = keyNames[event.key];
  if (key === undefined && /^F\d+$/.test(event.key)) {
    key = `[${event.key.toLowerCase()}]`;
  }
  if (key === undefined && [...event.key].length === 1) {
    key = event.key;
  }
  if (key !== undefined) {
    event.preventDefault();
    game.sendKey(key);
    advance();
  }
});

// A click anywhere on the screen is a click on the command line, unless
// the player is selecting text.
screen.addEventListener("click", () => {
  if (String(getSelection()) !== "") {
    return;
  }
  if (state === "line") {
    command.focus({ preventScroll: true });
  } else if (state === "key" && !textBelow()) {
    // Where there is no keyboard, a tap is the key to press.
    game.sendKey(" ");
    advance();
  }
});

// ---- the settings, and the about box ----

const settingsDialog = element("settings");

function showSettings() {
  for (const input of settingsDialog.querySelectorAll("[name]")) {
    const value = settings[input.name];
    if (input.type === "checkbox") {
      input.checked = Boolean(value);
    } else {
      input.value = String(value);
    }
  }
  settingsDialog.showModal();
}

settingsDialog.addEventListener("input", (event) => {
  const input = event.target;
  if (!input.name) {
    return;
  }
  settings[input.name] =
    input.type === "checkbox" ? input.checked : input.type === "range" ? Number(input.value) : input.value;
  saveSettings();
  applyStyles();
  if (input.name === "sounds" && !settings.sounds) {
    sounds.stop(null);
  }
  if (input.name === "music" && !settings.music) {
    sounds.stop("background");
  }
  if (game && rich) {
    game.setPlayerChoices(settings.pictures, settings.sounds, settings.music, settings.links);
  }
  if (input.name === "display") {
    element("display-note").hidden = false;
  }
});

element("settings-open").addEventListener("click", showSettings);
element("settings-close").addEventListener("click", () => settingsDialog.close());
element("settings-reset").addEventListener("click", () => {
  settings = { ...defaultSettings, display: settings.display };
  saveSettings();
  applyStyles();
  showSettings();
});

const aboutDialog = element("about-dialog");
element("about").addEventListener("click", () => aboutDialog.showModal());
element("about-close").addEventListener("click", () => aboutDialog.close());

// ---- loading a game ----

async function play(bytes, name) {
  gameName = name;
  sounds.stop(null);
  forgetResources();
  mainBuilder.reset();
  richPanes.clear();
  mapNames.clear();
  resetPaneLayout();
  panesTop.replaceChildren();
  panesBottom.replaceChildren();
  screen.removeAttribute("style");
  element("about").hidden = true;
  document.title = "Hexicon";
  notice.hidden = true;
  element("welcome").hidden = true;
  element("game-name").textContent = name;
  rich = settings.display !== "text";
  document.body.classList.toggle("rich", rich);
  try {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    game = new Game(bytes, name, zone);
    game.setFileDialogs(inPlayer);
  } catch (error) {
    game = null;
    state = "done";
    entry.hidden = true;
    say(`This does not look like a game Hexicon can play: ${error.message}`);
    return;
  }
  if (rich) {
    game.setRichDisplay(settings.pictures, settings.sounds, settings.music, settings.links);
  }
  applyStyles();
  // The game's sounds, decoded, ready to play.
  sounds.reset(rich && settings.sounds ? game.resourceNames() : []);
  element("saved-games").hidden = !inPlayer;
  await restoreFiles();
  unseen = null;
  fitToWindow();
  state = game.start();
  update();
}

element("file").addEventListener("change", async (event) => {
  const file = event.target.files[0];
  if (file) {
    play(new Uint8Array(await file.arrayBuffer()), file.name);
  }
  // Choosing the same file again plays it again.
  event.target.value = "";
});

// A game file dropped on the page plays, as one chosen does.
const draggingFiles = (event) => event.dataTransfer?.types.includes("Files");
document.addEventListener("dragover", (event) => {
  if (draggingFiles(event)) {
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    document.body.classList.add("dropping");
  }
});
document.addEventListener("dragleave", (event) => {
  // Leaving the window, not one of its parts.
  if (event.relatedTarget === null) {
    document.body.classList.remove("dropping");
  }
});
document.addEventListener("drop", async (event) => {
  if (!draggingFiles(event)) {
    return;
  }
  event.preventDefault();
  document.body.classList.remove("dropping");
  const file = event.dataTransfer.files[0];
  if (file) {
    play(new Uint8Array(await file.arrayBuffer()), file.name);
  }
});

// In the player, the folder of the game's files, in the system's file
// manager.
element("saved-games").addEventListener("click", () => {
  window.ipc.postMessage(`show ${encodeURIComponent(gameName)}`);
});
if (inPlayer) {
  element("welcome").innerHTML =
    "Choose a game file (<code>.hxg</code>), or drop one on the window, to play it. " +
    "Saved games are kept in your documents, in the folder Hexicon, with a folder " +
    "for each game: <b>Saved games</b> opens it.";
}

applyStyles();
await init();

// A page address can name a game to load: index.html?game=games/sample.hxg
const address = new URLSearchParams(location.search).get("game");
if (address) {
  try {
    const response = await fetch(address);
    if (!response.ok) {
      throw new Error(`${response.status} ${response.statusText}`);
    }
    await play(new Uint8Array(await response.arrayBuffer()), address.split("/").pop());
  } catch (error) {
    say(`The game at "${address}" could not be loaded: ${error.message}`);
  }
}
