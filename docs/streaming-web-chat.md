---
title: A tiny streaming web chat
description: Build a local browser chat on the Open Interpreter app-server protocol.
---

For a complete desktop, browser, or headless experience, use [Interpreter
Workstation](https://github.com/openinterpreter/interpreter-workstation) instead.
It already has a composer, approvals, sessions, and richer tool output. This
page is for learning how to put **your own small local UI** in front of raw
Open Interpreter (OIX), not for reproducing Workstation.

The browser talks to a loopback-only Node bridge; the bridge launches
`interpreter app-server --listen stdio://`. Your provider login/configuration
stays with the **server process**, never in JavaScript sent to the browser.
Sign in with `interpreter` first, then run these two files from a disposable
project directory. Node 20+ is needed; no npm packages are required.

The bridge sends JSON-RPC `initialize`, waits for its matching response, then
sends `initialized`. It calls `thread/start` once and `turn/start` for each
prompt. During that turn, `item/agentMessage/delta` carries text and
`turn/completed` carries the final status (including failures). The bridge
matches response IDs and thread/turn IDs before streaming text to the page.

Save the following two snippets as `bridge.mjs` and `index.html` in the same
directory, run `node bridge.mjs`, and open `http://127.0.0.1:8787/`.

<details>
<summary>Complete two-file local example</summary>

### `bridge.mjs`

```js
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";

const port = 8787;
const origin = `http://127.0.0.1:${port}`;
const child = spawn(
  process.env.OIX_BINARY ?? "interpreter",
  ["app-server", "--listen", "stdio://"],
  { stdio: ["pipe", "pipe", "inherit"] },
);
const pending = new Map();
let nextId = 0,
  threadId,
  active;
const write = (message) => child.stdin.write(JSON.stringify(message) + "\n");
function call(method, params) {
  const id = ++nextId;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    write({ id, method, params });
  });
}
function emit(a, event) {
  if (a.closed) return;
  const line = JSON.stringify(event) + "\n";
  // Reserve space for an explicit error; this is a per-turn transport limit,
  // not a limit on how many turns the app server can keep in its thread.
  if (a.bytes + Buffer.byteLength(line) > 65536 - 256) {
    fail(a, "Turn exceeded the 64 KiB browser stream limit; interrupted");
    return;
  }
  a.bytes += Buffer.byteLength(line);
  if (!a.res.write(line)) fail(a, "Browser too slow; turn interrupted");
}
function fail(a, reason) {
  if (a.closed) return;
  a.res.end(
    JSON.stringify({ type: "error", fatal: true, text: reason }) + "\n",
  );
  interrupt(a);
}
function interrupt(a) {
  a.closed = true;
  if (a.turnId && !a.interrupted) {
    a.interrupted = true;
    call("turn/interrupt", { threadId, turnId: a.turnId }).catch(() => {});
  }
}
function notification(message) {
  const { method, params: p } = message;
  const a = active;
  if (!a || p?.threadId !== threadId) return;
  if (a.closed) {
    if (method === "turn/completed" && (p.turnId ?? p.turn?.id) === a.turnId)
      active = undefined;
    return;
  }
  // Notifications may arrive before the turn/start *response*; queue only a
  // bounded number and bytes until its ID is known, then filter by both IDs.
  if (!a.turnId) {
    const size = Buffer.byteLength(JSON.stringify(message));
    if (a.queue.length < 32 && a.queueBytes + size <= 16384) {
      a.queue.push(message);
      a.queueBytes += size;
    } else fail(a, "Too many early events; turn interrupted");
    return;
  }
  if ((p.turnId ?? p.turn?.id) !== a.turnId) return;
  if (method === "item/agentMessage/delta")
    emit(a, { type: "text", text: p.delta });
  if (method === "error") emit(a, { type: "error", text: p.error.message });
  if (method === "turn/completed") {
    emit(a, {
      type: "done",
      status: p.turn.status,
      error: p.turn.error?.message ?? null,
    });
    if (!a.closed) a.res.end();
    a.closed = true;
    active = undefined;
  }
}
createInterface({ input: child.stdout }).on("line", (line) => {
  let m;
  try {
    m = JSON.parse(line);
  } catch {
    return;
  }
  if (m.id !== undefined && m.method) {
    // This teaching example has no approval UI. Explicitly decline requests;
    // never accept one just because a browser is connected.
    const approval = [
      "item/commandExecution/requestApproval",
      "item/fileChange/requestApproval",
    ].includes(m.method);
    write(
      approval
        ? { id: m.id, result: { decision: "decline" } }
        : {
            id: m.id,
            error: { code: -32601, message: "No UI for this request" },
          },
    );
    if (active) emit(active, { type: "error", text: `Declined: ${m.method}` });
  } else if (m.id !== undefined) {
    const p = pending.get(m.id);
    if (p) {
      pending.delete(m.id);
      m.error ? p.reject(Error(m.error.message)) : p.resolve(m.result);
    }
  } else if (m.method) notification(m);
});
child.on("error", (error) => {
  console.error(`Cannot launch interpreter: ${error.message}`);
  for (const p of pending.values()) p.reject(error);
  pending.clear();
});
child.stdin.on("error", () => {}); // a failed child may close stdin first
child.on("exit", () => {
  for (const p of pending.values()) p.reject(Error("app server exited"));
  pending.clear();
  if (active) fail(active, "app server exited");
  process.exitCode = 1;
});

// The app server requires the initialize response before initialized, then
// thread/start before turn/start. Keep a single read-only thread in this demo.
try {
  await call("initialize", {
    clientInfo: { name: "tiny-web-chat", version: "1.0" },
  });
  write({ method: "initialized" });
  threadId = (
    await call("thread/start", {
      cwd: process.cwd(),
      sandbox: "read-only",
      approvalPolicy: "on-request",
    })
  ).thread.id;
} catch (error) {
  console.error(`App server start failed: ${error.message}`);
  process.exit(1);
}

const server = createServer(async (req, res) => {
  if (
    req.headers.host !== `127.0.0.1:${port}` ||
    (req.method === "POST" && req.headers.origin !== origin)
  ) {
    res.writeHead(403).end();
    return;
  }
  if (req.method === "GET" && req.url === "/") {
    res.writeHead(200, {
      "Content-Type": "text/html; charset=utf-8",
      "X-Content-Type-Options": "nosniff",
    });
    res.end(readFileSync(new URL("./index.html", import.meta.url)));
    return;
  }
  if (req.method !== "POST" || req.url !== "/chat") {
    res.writeHead(404).end();
    return;
  }
  if (active) {
    res.writeHead(409).end("Wait for the current turn");
    return;
  }
  if (
    req.headers["content-type"] !== "text/plain" ||
    Number(req.headers["content-length"] || 0) > 4096
  ) {
    res.writeHead(413).end();
    return;
  }
  const chunks = [];
  let size = 0;
  try {
    for await (const chunk of req) {
      size += chunk.length;
      if (size > 4096) throw Error("Prompt too long");
      chunks.push(chunk);
    }
  } catch {
    res.writeHead(413).end();
    return;
  }
  const prompt = Buffer.concat(chunks).toString("utf8");
  if (!prompt.trim()) {
    res.writeHead(400).end();
    return;
  }
  // Both bodies may have started while idle; only one may reserve the turn.
  if (active) {
    res.writeHead(409).end("Wait for the current turn");
    return;
  }
  res.writeHead(200, {
    "Content-Type": "application/x-ndjson; charset=utf-8",
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
  });
  const a = (active = {
    res,
    queue: [],
    queueBytes: 0,
    bytes: 0,
    closed: false,
  });
  res.on("close", () => {
    if (active === a && !a.closed) interrupt(a);
  });
  try {
    a.turnId = (
      await call("turn/start", {
        threadId,
        input: [{ type: "text", text: prompt }],
      })
    ).turn.id;
    if (a.closed) interrupt(a);
    for (const event of a.queue) notification(event);
    a.queue.length = 0;
    a.queueBytes = 0;
  } catch (error) {
    fail(a, `Turn start failed: ${error.message}`);
    active = undefined;
  }
});
server.requestTimeout = 30_000;
server.listen(port, "127.0.0.1", () => console.log(origin));
```

### `index.html`

```html
<!doctype html>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Local Open Interpreter chat</title>
<style>
  body {
    max-width: 48rem;
    margin: 3rem auto;
    padding: 0 1rem;
    font: 1rem/1.5 system-ui;
    color: #20242c;
    background: #fafafa;
  }
  #chat {
    height: min(60vh, 32rem);
    overflow-y: auto;
    overflow-anchor: none;
    padding: 1rem;
    background: white;
    border: 1px solid #ddd;
    border-radius: 0.8rem;
  }
  #chat > div {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin-bottom: 1rem;
  }
  .state {
    color: #58606c;
    font-size: 0.85rem;
  }
  form {
    display: flex;
    gap: 0.6rem;
    margin-top: 1rem;
  }
  textarea {
    flex: 1;
    min-height: 4rem;
    font: inherit;
  }
</style>
<h1>Local chat</h1>
<div id="history" class="state" aria-live="polite"></div>
<div id="chat" role="log" aria-live="off"></div>
<form>
  <textarea aria-label="Your message" maxlength="4096" required></textarea>
  <button>Send</button>
</form>
<script>
  const form = document.querySelector("form"),
    box = document.querySelector("#chat");
  const history = document.querySelector("#history");
  const input = form.querySelector("textarea"),
    button = form.querySelector("button");
  const nearBottom = () =>
    box.scrollHeight - box.scrollTop - box.clientHeight < 48;
  function newTurn(prompt) {
    const pinned = nearBottom(),
      row = document.createElement("div");
    const user = document.createElement("div"),
      answer = document.createElement("div");
    const state = document.createElement("div");
    user.textContent = `You: ${prompt}`;
    answer.textContent = "Assistant: ";
    state.className = "state";
    state.textContent = "Streaming…";
    state.setAttribute("aria-live", "polite");
    row.append(user, answer, state);
    box.append(row);
    while (box.children.length > 8) {
      const oldest = box.firstElementChild,
        before = box.scrollHeight;
      const oldTop = box.scrollTop;
      oldest.remove(); // visible window only; the app-server thread is unchanged
      history.textContent =
        "Earlier visible turns trimmed (conversation stays on server).";
      if (!pinned)
        box.scrollTop = Math.max(0, oldTop + box.scrollHeight - before);
    }
    if (pinned) box.scrollTop = box.scrollHeight;
    return { answer, state };
  }
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    button.disabled = true;
    const prompt = input.value,
      { answer, state } = newTurn(prompt);
    let tail = "",
      trimmed = false,
      label = "Streaming…",
      frame = 0,
      reader;
    function paint() {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      const pinned = nearBottom();
      answer.textContent = `Assistant: ${trimmed ? "[Earlier answer text trimmed]\n" : ""}${tail}`;
      state.textContent = label;
      if (pinned) box.scrollTop = box.scrollHeight; // instant, only if following
    }
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(paint);
    };
    try {
      const response = await fetch("/chat", {
        method: "POST",
        headers: { "Content-Type": "text/plain" },
        body: prompt,
      });
      if (!response.ok) throw Error(`HTTP ${response.status}`);
      input.value = "";
      reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "",
        finished = false,
        fatal = false,
        notice = "";
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        if (buffer.length > 70000) throw Error("Stream line too large");
        let newline;
        while ((newline = buffer.indexOf("\n")) >= 0) {
          const event = JSON.parse(buffer.slice(0, newline));
          buffer = buffer.slice(newline + 1);
          if (event.type === "text") {
            if (tail.length + event.text.length > 12000) trimmed = true;
            tail = (tail + event.text).slice(-12000); // rolling visible tail
          } else if (event.type === "error") {
            if (event.fatal) {
              finished = true;
              fatal = true;
            } else notice = String(event.text).slice(0, 120);
            label = `${event.fatal ? "Interrupted" : "Notice"}: ${String(event.text).slice(0, 300)}`;
          } else if (event.type === "done") {
            finished = true;
            if (!fatal)
              label =
                event.status === "completed"
                  ? `Complete${notice ? ` (notice: ${notice})` : ""}`
                  : `${event.status}: ${String(event.error ?? "no details").slice(0, 300)}`;
          }
          schedule(); // at most one DOM update per animation frame
        }
      }
      if (decoder.decode() || buffer || !finished)
        throw Error("Stream closed without a complete final event");
    } catch (error) {
      label = `Error: ${String(error.message).slice(0, 300)}`;
      if (reader) await reader.cancel().catch(() => {});
    } finally {
      paint();
      button.disabled = false;
    }
  });
</script>
```

</details>

The same server home and working directory determine the account, provider
and workspace.
This deliberately supports **one local user and one turn at a time**; a second
POST gets `409`. It is not identity isolation: every local tab shares this
thread. Closing the response interrupts its turn; until the app server reports
`turn/completed`, another turn stays blocked. Each streamed
delta and completion is matched to the active thread and turn; JSON-RPC
responses are matched by request ID. Messages are rendered as text, never
HTML. This demo **declines** command/file approvals and rejects other server
requests; use Workstation or implement a real review UI if approvals are
needed. It does not display tool events, images, full conversation history, or
multiple users. The page keeps only the latest **eight visible turns** and the
last **12,000 characters of the current answer**; older visible text leaves the
top as the session grows. This is a small rolling display, **not** Workstation's
virtualized, retrievable history. If the text being read is trimmed, it cannot
be preserved; the same applies when the current answer's rolling tail replaces
text being read. Both trims are visibly marked. Otherwise the page keeps the
reader's position and follows new output only when already near the bottom.
The app server retains the thread's
conversation independently of this display. Browser writes are batched to one
animation frame, not animated scrolls.

The bridge's **64 KiB per-turn browser-stream cap** is an intentional safety
limit, not a total session limit. Hitting it interrupts the turn and visibly
reports an error; an unexpected EOF is never shown as success. A full client
would implement backpressure, pagination, and cancellation. This tiny process
starts a fresh thread on each launch; it does not implement persisted-session
resume or rebuild previous messages in the UI.

**Security boundary:** keep the bridge on loopback. Do not forward its port or
expose the raw app-server socket publicly. The Host/Origin checks and small
input/output bounds above are educational safeguards, **not** remote
authentication, CSRF protection for every deployment, rate limiting,
permission isolation, or production backpressure handling. For remote or
multi-user hosting you must add authenticated transport, explicit user/session
isolation, authorization, resource limits, and proper approval handling; do
not put tokens, OAuth files, or provider keys into the page. See [App Server](/docs/app-server),
[Sandbox & approvals](/docs/sandbox), and [Workstation's public source](https://github.com/openinterpreter/interpreter-workstation)
for full-client design.
