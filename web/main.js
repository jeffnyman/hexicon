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
}

// Show what the game has displayed, and get ready for what it needs next.
function update() {
  if (game.takeCleared()) {
    transcript.replaceChildren();
  }
  show(game.takeOutput());
  storeFiles();

  const left = game.status();
  const right = game.statusRight();
  element("status").hidden = left === "" && right === "";
  element("status-left").textContent = left;
  element("status-right").textContent = right;

  entry.hidden = state !== "line";
  if (state === "line") {
    command.focus();
  } else if (state === "sleep") {
    setTimeout(() => {
      game.wake();
      advance();
    }, game.sleepMillis());
  } else if (state === "done") {
    say("The game has ended. Choose a game to play again.");
  }
  screen.scrollTop = screen.scrollHeight;
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

document.addEventListener("keydown", (event) => {
  if (state !== "key" || event.ctrlKey || event.metaKey || event.altKey) {
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
  if (state === "line" && String(getSelection()) === "") {
    command.focus();
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
