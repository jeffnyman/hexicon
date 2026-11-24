// Play a game through the WebAssembly build under Node, reading commands
// from a file, one to a line, and writing what the game displays to
// standard output. This is for checking that the browser build plays a
// game the same way the command-line program does.
//
// Build first with: tools/build-web.sh nodejs
//
// usage: node tools/web-play.js <game.hxg> <commands.txt>
//
// Files the game writes, such as saved games, are kept in memory and are
// gone when the run ends.

const fs = require("fs");
const path = require("path");
const { Game } = require("../build/web-nodejs/hexicon_wasm.js");

const [imagePath, commandsPath] = process.argv.slice(2);
if (!imagePath || !commandsPath) {
  console.error("usage: node tools/web-play.js <game.hxg> <commands.txt>");
  process.exit(2);
}

const image = fs.readFileSync(imagePath);
const commands = fs
  .readFileSync(commandsPath, "utf8")
  .split("\n")
  .map((line) => line.replace(/\r$/, ""));
if (commands[commands.length - 1] === "") {
  commands.pop();
}

const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
const game = Game.withoutRandomness(image, path.basename(imagePath), zone);
let next = 0;
let state = game.start();
for (;;) {
  process.stdout.write(game.takeOutput());
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
    game.sendLine(commands[next++]);
  }
  state = game.resume();
}
