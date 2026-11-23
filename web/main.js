// A page that plays a game with Hexicon.
//
// The game runs until it needs something from the player, then stops and
// says what: a line of input, a keystroke, or a pause. The page supplies
// it and tells the game to carry on.

import init, { Game } from "./pkg/hexicon_wasm.js";

const element = (id) => document.getElementById(id);
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
// banners and its main window share them.
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
    showBanners();
  }
  sizeCommandLine();
}

// ---- banners ----

const bannersTop = element("banners-top");
const bannersBottom = element("banners-bottom");

// Show the game's banners: those above its main window in a strip above
// the text, and those below in a strip below it.
function showBanners() {
  const layout = JSON.parse(game.banners());
  const mainTop = layout.main.y;
  const mainBottom = mainTop + layout.main.height;
  fillStrip(
    bannersTop,
    layout.banners.filter((banner) => banner.y < mainTop),
    0,
    mainTop,
  );
  fillStrip(
    bannersBottom,
    layout.banners.filter((banner) => banner.y >= mainBottom),
    mainBottom,
    rows - mainBottom,
  );
}

function fillStrip(strip, banners, firstRow, height) {
  const visible = banners.filter((banner) => banner.width > 0 && banner.height > 0);
  strip.hidden = height <= 0 || visible.length === 0;
  strip.style.height = `${height * lineHeight()}px`;
  strip.replaceChildren(
    ...visible.map((banner) => {
      const pane = document.createElement("div");
      pane.className = banner.status ? "banner status" : "banner";
      pane.textContent = banner.lines.join("\n");
      pane.style.top = `${(banner.y - firstRow) * lineHeight()}px`;
      pane.style.height = `${banner.height * lineHeight()}px`;
      // The text lines up with the main window's, and a banner at an edge
      // of the screen reaches the edge of the page.
      if (banner.x === 0) {
        pane.style.left = "0";
        pane.style.paddingLeft = getComputedStyle(screen).paddingLeft;
      } else {
        pane.style.left = `calc(${getComputedStyle(screen).paddingLeft} + ${banner.x}ch)`;
      }
      if (banner.x + banner.width >= columns + 1) {
        pane.style.right = "0";
      } else {
        pane.style.width = `${banner.width}ch`;
      }
      return pane;
    }),
  );
}

// The command line takes up the rest of the line its prompt is on.
function sizeCommandLine() {
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
  if (unseen && unseen.isConnected) {
    const start =
      unseen.getBoundingClientRect().top - screen.getBoundingClientRect().top + screen.scrollTop;
    top = Math.min(bottom, Math.max(0, start - lineHeight() / 2));
  }
  screen.scrollTop = top;
  unseen = null;
  showMore();
}

// Show what the game has displayed, and get ready for what it needs next.
function update() {
  if (game.takeCleared()) {
    transcript.replaceChildren();
    unseen = null;
  }
  show(game.takeOutput());
  storeFiles();
  showBanners();

  const left = game.status();
  const right = game.statusRight();
  element("status").hidden = left === "" && right === "";
  element("status-left").textContent = left;
  element("status-right").textContent = right;

  entry.hidden = state !== "line";
  sizeCommandLine();
  if (state === "line") {
    command.focus({ preventScroll: true });
  } else if (state === "sleep") {
    setTimeout(() => {
      game.wake();
      advance();
    }, game.sleepMillis());
  } else if (state === "done") {
    say("The game has ended. Choose a game to play again.");
  }
  scrollToNewText();
}

function advance() {
  state = game.resume();
  update();
}

// ---- the player ----

entry.addEventListener("submit", (event) => {
  event.preventDefault();
  if (state !== "line") {
    return;
  }
  const line = command.value;
  command.value = "";
  // The game does not display what the player types; a terminal would.
  show(line + "\n", "typed");
  game.sendLine(line);
  advance();
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

document.addEventListener("keydown", (event) => {
  if (state !== "key" || event.ctrlKey || event.metaKey || event.altKey) {
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

// ---- loading a game ----

function play(bytes, name) {
  gameName = name;
  transcript.replaceChildren();
  notice.hidden = true;
  element("welcome").hidden = true;
  element("game-name").textContent = name;
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

await init();

// A page address can name a game to load: index.html?game=games/sample.t3
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
