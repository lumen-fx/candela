// The browser half of the `browser` package: answers the functions the
// package's `host "browser"` block declares.
//
// The embedder calls `install` once with the program's event queue, then
// binds each exported function to the host function of the same name. A
// function that answers later takes a source first, posts
// `{"value": ...}` or `{"error": message}` under it once, and releases it.
// A listener posts under its source for as long as the page lives.

let candela;
const popups = new Map();
const watched = new Map();
let unloadGuard = null;
const messageSources = [];
const fullscreenSources = [];

export function install(runtime) {
  candela = runtime;
  window.addEventListener("message", (event) => {
    // A message whose data JSON cannot write, such as one another script on
    // the page sent with a transferable in it, is not the program's.
    let data;
    try {
      data = JSON.parse(JSON.stringify(event.data ?? null));
    } catch {
      return;
    }
    const text = JSON.stringify({
      origin: event.origin,
      data,
      source: sourceName(event.source),
    });
    for (const source of messageSources) {
      candela.post(source, text);
    }
  });
  document.addEventListener("fullscreenchange", () => {
    const element = document.fullscreenElement;
    const text = JSON.stringify({ id: element?.id ?? "", on: element !== null });
    for (const source of fullscreenSources) {
      candela.post(source, text);
    }
  });
}

// Posts what `promise` settles to under `source`, then lets the source go.
function answer(source, promise) {
  candela.hold(source);
  Promise.resolve(promise).then(
    (value) => candela.post(source, JSON.stringify({ value })),
    (error) => candela.post(source, JSON.stringify({ error: String(error?.message ?? error) })),
  ).finally(() => candela.release(source));
}

// What a script calls the window a message came from.
function sourceName(source) {
  if (source === null) return "";
  if (source === window.opener) return "opener";
  if (source === window.parent && window.parent !== window) return "parent";
  for (const [name, popup] of popups) {
    if (popup === source) return name;
  }
  return "";
}

// The window a script names: `opener`, `parent`, or a popup it opened.
function windowNamed(target) {
  if (target === "opener") return window.opener;
  if (target === "parent") return window.parent !== window ? window.parent : null;
  const popup = popups.get(target);
  return popup && !popup.closed ? popup : null;
}

// The browser opens a picker, a fullscreen view or a share sheet only right
// after the visitor did something. Asking without that is refused here with a
// reason, where the browser would ignore it and never answer.
function needsGesture(what) {
  if (navigator.userActivation && !navigator.userActivation.isActive) {
    throw new Error(`${what} needs a click or key press just before it`);
  }
}

export function candela_browser_open_popup(url, name, features) {
  const popup = window.open(new URL(url, document.baseURI).href, name, features);
  if (!popup) return false;
  popups.set(name, popup);
  return true;
}

export function candela_browser_close_popup(name) {
  const popup = popups.get(name);
  popups.delete(name);
  if (!popup || popup.closed) return false;
  popup.close();
  return true;
}

export function candela_browser_post_message(target, data, origin) {
  const to = windowNamed(target);
  if (!to) return false;
  to.postMessage(data, origin);
  return true;
}

export function candela_browser_close_window() {
  window.close();
}

export function candela_browser_share(source, data) {
  answer(
    source,
    (async () => {
      if (typeof navigator.share !== "function") {
        throw new Error("this browser cannot share");
      }
      needsGesture("sharing");
      await navigator.share(data ?? {});
      return true;
    })(),
  );
}

export function candela_browser_can_share(data) {
  return typeof navigator.canShare === "function" && navigator.canShare(data ?? {});
}

export function candela_browser_download_text(filename, mime, text) {
  const url = URL.createObjectURL(new Blob([text], { type: mime || "text/plain" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.style.display = "none";
  document.body.append(link);
  link.click();
  link.remove();
  // The download has its own reference by the time the click returns; the
  // URL only has to outlive the task that started it.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

export function candela_browser_guard_unload(on) {
  if (on && !unloadGuard) {
    unloadGuard = (event) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", unloadGuard);
  } else if (!on && unloadGuard) {
    window.removeEventListener("beforeunload", unloadGuard);
    unloadGuard = null;
  }
}

function mediaList(query) {
  let list = watched.get(query);
  if (!list) {
    list = window.matchMedia(query);
    watched.set(query, list);
  }
  return list;
}

export function candela_browser_match_media(query) {
  return mediaList(query).matches;
}

export function candela_browser_watch_media(source, query) {
  mediaList(query).addEventListener("change", (event) => {
    candela.post(source, JSON.stringify({ matches: event.matches }));
  });
}

export function candela_browser_pick_file(source, accept) {
  answer(
    source,
    new Promise((resolve, reject) => {
      needsGesture("picking a file");
      const input = document.createElement("input");
      input.type = "file";
      input.accept = accept;
      input.addEventListener("change", () => {
        const file = input.files && input.files[0];
        if (!file) {
          reject(new Error("no file was picked"));
          return;
        }
        file.text().then(
          (text) => resolve({ name: file.name, type: file.type, size: file.size, text }),
          reject,
        );
      });
      input.addEventListener("cancel", () => reject(new Error("no file was picked")));
      input.click();
    }),
  );
}

export function candela_browser_request_fullscreen(source, id) {
  answer(
    source,
    (async () => {
      needsGesture("going fullscreen");
      const element = id === "" ? document.documentElement : document.getElementById(id);
      if (!element) {
        throw new Error(`no element has id="${id}"`);
      }
      await element.requestFullscreen();
      return true;
    })(),
  );
}

export function candela_browser_exit_fullscreen() {
  if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => {});
  }
}

export function candela_browser_is_fullscreen() {
  return document.fullscreenElement !== null;
}

export function candela_browser_on_message(source) {
  messageSources.push(source);
}

export function candela_browser_on_fullscreen_change(source) {
  fullscreenSources.push(source);
}
