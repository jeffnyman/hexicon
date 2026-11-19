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
//
// The page address names the game to load:
//   http://127.0.0.1:8765/web/index.html?game=/build/web-test/sample.t3
// The commands file has one command to a line.

const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const [browserPath, address, commandsPath] = process.argv.slice(2);
if (!browserPath || !address || !commandsPath) {
  console.error(
    "usage: node tools/web-check.js <browser.exe> <page address> <commands.txt>",
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
const browser = spawn(
  browserPath,
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
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
  await send("Page.navigate", { url: address });
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
  const files = await evaluate(
    `Object.keys(localStorage).filter((key) => key.startsWith("hexicon:")).join(", ")`,
  );
  if (files) {
    process.stderr.write(`stored: ${files}\n`);
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
