// Play a game through the WebAssembly build under Node, reading commands
// from a file, one to a line, and writing what the game displays to
// standard output. This is for checking that the browser build plays a
// game the same way the command-line program does.
//
// Build first with: tools/build-web.sh nodejs
//
// usage: node tools/web-play.js [--rich] <game.hxg> <commands.txt>
//
// With --rich, the game is played as on a rich display, and what is
// written is the checked document as text, as `hexicon run --rich` writes
// it, each command as the line the player typed (<kbd>look</kbd>).
// The commands file may be an event script (walk.txt): its <line> tags
// are taken off.
//
// Files the game writes, such as saved games, are kept in memory and are
// gone when the run ends.

const fs = require("fs");
const path = require("path");
const { Game } = require("../build/web-nodejs/hexicon_wasm.js");

const args = process.argv.slice(2);
const rich = args[0] === "--rich";
if (rich) {
  args.shift();
}
const [imagePath, commandsPath] = args;
if (!imagePath || !commandsPath) {
  console.error("usage: node tools/web-play.js [--rich] <game.hxg> <commands.txt>");
  process.exit(2);
}

const image = fs.readFileSync(imagePath);
const commands = fs
  .readFileSync(commandsPath, "utf8")
  .split("\n")
  .map((line) => line.replace(/\r$/, ""))
  .filter((line) => line.trim() !== "<eventscript>")
  .map((line) => line.replace(/^<line>/, ""));
if (commands[commands.length - 1] === "") {
  commands.pop();
}

// The same escaping the checker gives text.
const escape = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/\u00a0/g, "&nbsp;");

const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
const game = Game.withoutRandomness(image, path.basename(imagePath), zone);
if (rich) {
  game.setRichDisplay(true, true, true, true);
}
const take = () => (rich ? game.takeDocumentText() : game.takeOutput());
let next = 0;
let state = game.start();
for (;;) {
  process.stdout.write(take());
  if (state === "done") {
    break;
  }
  if (state === "sleep") {
    // A test has no reason to wait.
    game.wake();
  } else if (next >= commands.length) {
    // Out of commands: nothing more will happen.
    break;
  } else if (state === "key") {
    const line = commands[next++];
    game.sendKey(line === "" ? "\n" : line[0]);
  } else {
    const line = commands[next++];
    if (rich) {
      process.stdout.write(`<kbd>${escape(line)}</kbd>\n`);
    }
    game.sendLine(line);
  }
  state = game.resume();
}
