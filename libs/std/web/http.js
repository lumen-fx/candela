// The browser half of `std/http`: the page's own fetch, answering the
// functions the module's `host "http"` block declares.
//
// The embedder calls `install` once with the program's event queue, then
// binds each exported function to the host function of the same name. A
// request answers once, in the shape `event::answer` reads: `{"value":
// {status, headers, body}}` or `{"error": message}`, then releases its source.

let candela;
const pending = new Map();

export function install(runtime) {
  candela = runtime;
}

function answer(source, value) {
  if (!pending.delete(source)) {
    return;
  }
  candela.post(source, JSON.stringify(value));
  candela.release(source);
}

export function candela_http_request(source, url, method, headersJson, body, timeoutMs, maxBytes) {
  candela.hold(source);
  const controller = new AbortController();
  pending.set(source, controller);
  let headers;
  try {
    headers = headersJson ? JSON.parse(headersJson) : {};
  } catch (error) {
    answer(source, { error: `${url}: the headers are not a JSON object` });
    return;
  }
  const init = { method: String(method).toUpperCase(), headers, signal: controller.signal };
  if (body !== "" && init.method !== "GET" && init.method !== "HEAD") {
    init.body = body;
  }
  const timer = timeoutMs > 0 ? setTimeout(() => controller.abort("timeout"), timeoutMs) : null;
  (async () => {
    const reply = await fetch(new URL(url, document.baseURI).href, init);
    const text = await reply.text();
    if (maxBytes > 0 && new TextEncoder().encode(text).length > maxBytes) {
      throw new Error(`the body passed the ${maxBytes} byte limit`);
    }
    const names = {};
    reply.headers.forEach((value, name) => {
      names[name.toLowerCase()] = value;
    });
    return { status: reply.status, headers: names, body: text };
  })().then(
    (value) => answer(source, { value }),
    (error) => answer(source, { error: `${url}: ${controller.signal.aborted ? controller.signal.reason : error}` }),
  ).finally(() => {
    if (timer !== null) {
      clearTimeout(timer);
    }
  });
}

export function candela_http_cancel(source) {
  const controller = pending.get(source);
  if (controller === undefined) {
    return 0;
  }
  answer(source, { error: "cancelled" });
  controller.abort("cancelled");
  return 1;
}
