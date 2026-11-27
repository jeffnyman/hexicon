// Play a game on the web page in a real browser, without a window, and
// write what the page shows to standard output. This checks the page
// itself: that it loads the WebAssembly build, shows the game's text, and
// takes commands.
//
// It needs Chrome or Edge, and Node 22 or later. Build the page first
// with tools/build-web.sh (or build-web.ps1), and serve the repository
// over HTTP with: node tools/web-serve.js
//
// usage: node tools/web-check.js <browser.exe> <page address> <commands.txt>
//        node tools/web-check.js --player <hexicon-player> <game.hxg> <commands.txt>
//
// The page address names the game to load:
//   http://127.0.0.1:8765/web/index.html?game=/build/web-test/sample.hxg
// The commands file has one command to a line.
//
// With --player, the page checked is the one in Hexicon's player (built by
// tools/build-app.sh), playing the game named, in the system's web view.
// This is for Windows, where the web view is WebView2, whose debugging
// port is turned on from outside. The player's window shows while it runs.
//
// These environment variables change what is checked:
//   HEXICON_CHECK_SIZE=390x800   the size of the window, for trying a
//                                narrow one
//   HEXICON_CHECK_PICTURE=a.png  save a picture of the page at the end
//   HEXICON_CHECK_EVAL=js        print what this gives in the page, such
//                                as the sizes of its panes
//   HEXICON_CHECK_BEFORE_PICTURE=js  run this in the page first, such as
//                                a click that opens a dialog

const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const args = process.argv.slice(2);
const player = args[0] === "--player";
if (player) {
  args.shift();
}
const [browserPath, address, commandsPath] = args;
if (!browserPath || !address || !commandsPath) {
  console.error(
    "usage: node tools/web-check.js <browser.exe> <page address> <commands.txt>\n" +
      "       node tools/web-check.js --player <hexicon-player> <game.hxg> <commands.txt>",
  );
  process.exit(2);
}
const commands = fs
  .readFileSync(commandsPath, "utf8")
  .split("\n")
  .map((line) => line.replace(/\r$/, ""));
if (commands[commands.length - 1] === "") {
  commands.pop();
}

const port = 9333;
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "hexicon-web-check-"));
const browser = player
  ? spawn(browserPath, [address], {
      stdio: "ignore",
      env: {
        ...process.env,
        // Sound may play before the player has done anything, so that a
        // check can see it playing.
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --autoplay-policy=no-user-gesture-required`,
      },
    })
  : spawn(
  browserPath,
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    `--window-size=${(process.env.HEXICON_CHECK_SIZE ?? "1200x900").replace("x", ",")}`,
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    "--autoplay-policy=no-user-gesture-required",
    "about:blank",
  ],
  { stdio: "ignore" },
);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The browser's debugging interface: requests and replies over a socket.
async function connect() {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = pages.find((entry) => entry.type === "page");
      if (page) {
        const socket = new WebSocket(page.webSocketDebuggerUrl);
        await new Promise((resolve, reject) => {
          socket.onopen = resolve;
          socket.onerror = reject;
        });
        return socket;
      }
    } catch {
      // The browser is not listening yet.
    }
    await sleep(100);
  }
  throw new Error("the browser did not start");
}

async function main() {
  const socket = await connect();
  let nextId = 0;
  const waiting = new Map();
  const problems = [];
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (waiting.has(message.id)) {
      waiting.get(message.id)(message);
      waiting.delete(message.id);
    } else if (message.method === "Runtime.exceptionThrown") {
      // What went wrong in the page, for when it does.
      const details = message.params.exceptionDetails;
      problems.push(details.exception?.description ?? details.text);
    } else if (message.method === "Log.entryAdded") {
      problems.push(
        `${message.params.entry.text} ${message.params.entry.url ?? ""}`,
      );
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve) => {
      const id = ++nextId;
      waiting.set(id, resolve);
      socket.send(JSON.stringify({ id, method, params }));
    });
  // Run JavaScript in the page and return its value.
  const evaluate = async (expression) => {
    const reply = await send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (reply.result.exceptionDetails) {
      throw new Error(JSON.stringify(reply.result.exceptionDetails));
    }
    return reply.result.result.value;
  };
  // Wait for something to be true of the page.
  const until = async (expression, what) => {
    for (let attempt = 0; attempt < 300; attempt++) {
      if (await evaluate(expression)) {
        return;
      }
      await sleep(50);
    }
    const showing = await evaluate(`document.body.innerText`);
    throw new Error(
      [
        `timed out waiting for ${what}`,
        ...problems,
        "the page shows:",
        showing,
      ].join("\n  "),
    );
  };

  await send("Page.enable");
  await send("Runtime.enable");
  await send("Log.enable");
  if (!player) {
    await send("Page.navigate", { url: address });
  }
  const ready = `!document.getElementById("entry").hidden || !document.getElementById("notice").hidden`;
  await until(ready, "the game to start");

  for (const command of commands) {
    if (await evaluate(`document.getElementById("entry").hidden`)) {
      break;
    }
    // Type the command and press Enter, and wait for the page to have
    // taken it: the command line is empty again once it has.
    await evaluate(`(() => {
      const command = document.getElementById("command");
      command.value = ${JSON.stringify(command)};
      document.getElementById("entry").requestSubmit();
    })()`);
    await until(
      `document.getElementById("command").value === ""`,
      "a command to be taken",
    );
  }

  process.stdout.write(
    await evaluate(`document.getElementById("transcript").textContent`),
  );
  const notice = await evaluate(
    `document.getElementById("notice").hidden ? "" : document.getElementById("notice").textContent`,
  );
  if (notice) {
    process.stdout.write(`\n[${notice}]\n`);
  }
  const status = await evaluate(
    `document.getElementById("status").hidden ? "" : document.getElementById("status-left").textContent + " | " + document.getElementById("status-right").textContent`,
  );
  if (status) {
    process.stderr.write(`status line: ${status}\n`);
  }
  const longest = await evaluate(
    `Math.max(...document.getElementById("transcript").textContent.split("\\n").map((line) => line.length))`,
  );
  const sideways = await evaluate(
    `document.getElementById("screen").scrollWidth > document.getElementById("screen").clientWidth`,
  );
  process.stderr.write(
    `longest line: ${longest} characters; scrolls sideways: ${sideways ? "yes" : "no"}\n`,
  );
  if (process.env.HEXICON_CHECK_EVAL) {
    // Something to ask the page, such as the size of its panes.
    process.stdout.write(`eval: ${await evaluate(process.env.HEXICON_CHECK_EVAL)}
`);
  }
  if (process.env.HEXICON_CHECK_BEFORE_PICTURE) {
    // Something to do to the page first, such as opening a dialog.
    await evaluate(process.env.HEXICON_CHECK_BEFORE_PICTURE);
    await sleep(300);
  }
  if (process.env.HEXICON_CHECK_PICTURE) {
    const picture = await send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(process.env.HEXICON_CHECK_PICTURE, Buffer.from(picture.result.data, "base64"));
  }
  // What is playing, if the page says.
  const playing = await evaluate(
    `JSON.stringify(window.hexiconSoundsPlaying ? window.hexiconSoundsPlaying() : [])`,
  );
  if (playing !== "[]") {
    process.stderr.write(`sounds: ${playing}
`);
  }
  const files = await evaluate(
    `Object.keys(localStorage).filter((key) => key.startsWith("hexicon:")).join(", ")`,
  );
  if (files) {
    process.stderr.write(`stored: ${files}\n`);
  }
  for (const problem of problems) {
    process.stderr.write(`problem: ${problem}\n`);
  }
  socket.close();
}

main()
  .catch((error) => {
    console.error(String(error));
    process.exitCode = 1;
  })
  .finally(() => {
    browser.kill();
  });
