/**
 * T342 — as much of WebDriver as the harness actually uses, and not a byte more.
 *
 * **Why by hand rather than a library.** The protocol here is JSON over HTTP, and what the
 * harness does with it is five verbs: open a session, find an element, read its text, click
 * it, close the session. A WebDriver client brings a hundred transitive packages, and every
 * one of them has to pass the licence check and be listed in `THIRD-PARTY.md` — a large
 * standing cost for eighty lines of `fetch`.
 *
 * The same reasoning as the walk from the root name servers (R-23 in spirit): the surface is
 * small, the correctness is checkable, and the dependency would outlive the need.
 *
 * Errors carry the server's own words. A WebDriver failure that reaches a person as
 * "request failed" is a failure they cannot act on; the protocol puts a real message in the
 * body, and it is passed through.
 */

/** How long to wait for an element before giving up, in milliseconds. */
const PATIENCE_MS = 15_000;

/** How often to look again while waiting. */
const LOOK_EVERY_MS = 250;

interface Answer<T> {
  value: T;
}

export class WebDriverError extends Error {
  constructor(
    readonly status: number,
    readonly body: string,
  ) {
    super(`WebDriver answered ${status}: ${body.slice(0, 400)}`);
    this.name = "WebDriverError";
  }
}

async function call<T>(url: string, method: "GET" | "POST" | "DELETE", body?: unknown): Promise<T> {
  const answer = await fetch(url, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await answer.text();
  if (!answer.ok) throw new WebDriverError(answer.status, text);
  // A DELETE answers with an empty value; JSON.parse on "" throws, and that would turn a
  // successful close into a failure.
  return (text ? (JSON.parse(text) as Answer<T>).value : (undefined as T)) as T;
}

/** The handle WebDriver returns for an element, under its long-winded protocol key. */
const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";

export class Session {
  private constructor(
    private readonly base: string,
    readonly id: string,
  ) {}

  /**
   * Open the application and start driving it.
   *
   * `application` is the built binary — not the installer and not `tauri dev`. What is
   * checked has to be what is handed out.
   */
  static async open(port: number, application: string): Promise<Session> {
    const base = `http://127.0.0.1:${port}`;
    const value = await call<{ sessionId: string }>(`${base}/session`, "POST", {
      capabilities: { alwaysMatch: { "tauri:options": { application } } },
    });
    return new Session(base, value.sessionId);
  }

  private at(path: string) {
    return `${this.base}/session/${this.id}${path}`;
  }

  /** Find one element by CSS, waiting for it to appear. */
  async find(css: string): Promise<Element> {
    const until = Date.now() + PATIENCE_MS;
    let last: unknown;
    for (;;) {
      try {
        const value = await call<Record<string, string>>(this.at("/element"), "POST", {
          using: "css selector",
          value: css,
        });
        return new Element(this, value[ELEMENT_KEY]);
      } catch (e) {
        last = e;
        if (Date.now() >= until) break;
        await new Promise((r) => setTimeout(r, LOOK_EVERY_MS));
      }
    }
    throw new Error(
      `no element matched ${css} within ${PATIENCE_MS / 1000}s. Last answer: ${String(last)}`,
    );
  }

  /** Whether an element matching this is on the page right now. No waiting. */
  async has(css: string): Promise<boolean> {
    const found = await call<unknown[]>(this.at("/elements"), "POST", {
      using: "css selector",
      value: css,
    });
    return found.length > 0;
  }

  /** The whole page's text. Used for saying what was on screen when something failed. */
  async pageText(): Promise<string> {
    const body = await this.find("body");
    return await body.text();
  }

  async elementText(handle: string): Promise<string> {
    return await call<string>(this.at(`/element/${handle}/text`), "GET");
  }

  async click(handle: string): Promise<void> {
    await call<null>(this.at(`/element/${handle}/click`), "POST", {});
  }

  async close(): Promise<void> {
    await call<null>(this.at(""), "DELETE");
  }
}

export class Element {
  constructor(
    private readonly session: Session,
    readonly handle: string,
  ) {}

  text() {
    return this.session.elementText(this.handle);
  }

  click() {
    return this.session.click(this.handle);
  }
}
