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

// ---- the game's files, kept in the browser's storage ----

const storagePrefix = () => `hexicon:${gameName}:`;

const toBase64 = (bytes) => {
  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return btoa(text);
};

const fromBase64 = (text) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

// Give the game the files it wrote when it was last played here.
function restoreFiles() {
  const prefix = storagePrefix();
  for (let index = 0; index < localStorage.length; index++) {
    const key = localStorage.key(index);
    if (key.startsWith(prefix)) {
      game.putFile(key.slice(prefix.length), fromBase64(localStorage.getItem(key)));
    }
  }
}

// Store the files the game has written, and forget the ones it deleted.
function storeFiles() {
  for (const name of game.takeChangedFiles()) {
    const contents = game.getFile(name);
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
      player += rule("#game #screen a[data-command], #game #screen .hx-input, #game #screen kbd", [
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
const sounds = {
  layers: new Map(),
  // Sounds the browser would not play before the player did anything.
  waiting: [],

  layer(name) {
    if (!this.layers.has(name)) {
      this.layers.set(name, new Set());
    }
    return this.layers.get(name);
  },

  stop(name) {
    const names = name ? [name] : [...this.layers.keys()];
    for (const layerName of names) {
      for (const entry of this.layer(layerName)) {
        entry.audio.pause();
        clearInterval(entry.timer);
      }
      this.layer(layerName).clear();
    }
    this.waiting = this.waiting.filter((entry) => !names.includes(entry.layer));
  },

  start(entry) {
    entry.audio.play().catch(() => {
      if (!this.waiting.includes(entry)) {
        this.waiting.push(entry);
      }
    });
  },

  // The browser lets a page play sounds once the player has done
  // something on it; those waiting for that start now.
  resume() {
    const waiting = this.waiting;
    this.waiting = [];
    for (const entry of waiting) {
      if (this.layer(entry.layer).has(entry)) {
        this.start(entry);
      }
    }
  },

  // A <sound> tag.
  handle(attrs) {
    if ("cancel" in attrs) {
      this.stop(attrs.cancel ? attrs.cancel.toLowerCase() : null);
      return;
    }
    const layer = (attrs.layer ?? "foreground").toLowerCase();
    const music = layer === "background";
    if (!settings.sounds || (music && !settings.music) || !attrs.src) {
      return;
    }
    const url = resourceUrl(attrs.src);
    if (!url) {
      return;
    }
    if ("interrupt" in attrs) {
      this.stop(layer);
    }
    const audio = new Audio(url);
    const entry = { audio, layer, timer: null };
    this.layer(layer).add(entry);
    const repeat = (attrs.repeat ?? "1").toLowerCase();
    let left = repeat === "loop" ? Infinity : Math.max(1, parseInt(repeat, 10) || 1);
    const chance = attrs.random !== undefined ? parseFloat(attrs.random) : NaN;
    if (!Number.isNaN(chance)) {
      // Now and then: each second, this chance in a hundred that it
      // plays, if it is not playing already.
      entry.timer = setInterval(() => {
        if (audio.paused && left > 0 && Math.random() * 100 < chance) {
          left -= 1;
          this.start(entry);
        }
      }, 1000);
      return;
    }
    audio.addEventListener("ended", () => {
      left -= 1;
      if (left > 0) {
        this.start(entry);
      } else {
        this.layer(layer).delete(entry);
      }
    });
    this.start(entry);
  },
};

// ---- the rich display: the document the game makes ----

// Reads the game's checked items into the elements of one window: the
// main window, or a pane. Elements are opened and closed as the items
// say; the checker has put them in order.
class Builder {
  constructor(root) {
    this.root = root;
    this.stack = [{ name: "#root", node: root }];
    // A <tab align=right>: what follows, to the end of the line, goes to
    // the right.
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
    unseen ??= node;
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
      case "tab":
        if ((attrs.align ?? "").toLowerCase() === "right") {
          this.endRight();
          const right = document.createElement("span");
          right.className = "hx-right";
          this.append(right);
          this.stack.push({ name: "#right", node: right });
          this.rightOf = right;
        }
        return;
      case "basefont":
        return;
      case "br":
        this.endRight();
        break;
      default:
        break;
    }
    this.append(makeElement(name, attrs));
  }

  // The end of the line ends what a <tab align=right> put at the right.
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
  // about box's.
  body(attrs) {
    const about = this.stack.find((entry) => entry.about);
    const target = about ? about.node : this.root === transcript ? screen : this.root;
    if (attrs.bgcolor) {
      target.style.backgroundColor = colorOf(attrs.bgcolor);
    }
    if (attrs.text) {
      target.style.color = colorOf(attrs.text);
    }
    if (attrs.link) {
      target.style.setProperty("--hx-link", colorOf(attrs.link));
    }
    if (attrs.background) {
      const url = resourceUrl(attrs.background);
      target.style.backgroundImage = url ? `url("${url}")` : "";
    }
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
      case "asrc":
      case "hsrc":
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
  if (name === "img" && node.dataset.missing !== undefined) {
    // A picture the game does not have: its words, if any.
    const words = document.createElement("span");
    words.textContent = attrs.alt ?? "";
    return words;
  }
  return node;
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

// Show the game's panes: those above its main window in a strip above
// the text, and those below in a strip below it.
function showPanes() {
  const layout = JSON.parse(game.panes());
  const mainTop = layout.main.y;
  const mainBottom = mainTop + layout.main.height;
  const above = layout.panes.filter((pane) => pane.y < mainTop);
  const below = layout.panes.filter((pane) => pane.y >= mainBottom);
  if (rich) {
    fillRichStrip(panesTop, above);
    fillRichStrip(panesBottom, below);
    return;
  }
  fillStrip(panesTop, above, 0, mainTop);
  fillStrip(panesBottom, below, mainBottom, rows - mainBottom);
}

// On a rich display, a text pane shows its document at the size of what
// is in it; a text grid, its lines.
function fillRichStrip(strip, panes) {
  const shown = panes
    .filter((pane) => pane.width > 0 && pane.height > 0)
    .map((pane) => {
      const builder = richPanes.get(pane.id);
      if (builder) {
        return builder.root;
      }
      const box = document.createElement("div");
      box.className = pane.status ? "grid-pane status" : "grid-pane";
      box.textContent = pane.lines.join("\n");
      return box;
    });
  strip.hidden = shown.length === 0;
  strip.style.height = "";
  strip.replaceChildren(...shown);
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
    command.style.width = "";
    return;
  }
  const text = transcript.textContent;
  const prompt = text.length - (text.lastIndexOf("\n") + 1);
  command.style.width = `${Math.max(columns - prompt - 1, 8)}ch`;
}

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

function play(bytes, name) {
  gameName = name;
  sounds.stop(null);
  forgetResources();
  mainBuilder.reset();
  richPanes.clear();
  mapNames.clear();
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
  restoreFiles();
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
    play(new Uint8Array(await response.arrayBuffer()), address.split("/").pop());
  } catch (error) {
    say(`The game at "${address}" could not be loaded: ${error.message}`);
  }
}
