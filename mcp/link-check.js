// grooph-link's parser as a WebAssembly module (crates/link-check), so the
// tool checks its link by the rules the app opens it with. The module has no
// imports and four exports: `buffer(len)` for the query, `check()` returning
// the length of the message, `message()` where it is, and `opened()`: whether
// the message says what the link opens rather than why it does not.

/** `checkLink(query)`: `{ problem }`, why the link with this query (without
 * `?`) would not open in the app, or `{ description }`, what it opens, as the
 * user sees it: meter and grouping, tempo, click bar, polyrhythm, swing.
 * `module` is the compiled module or its bytes. */
export function linkChecker(module) {
  const compiled = module instanceof WebAssembly.Module ? module : new WebAssembly.Module(module);
  const { memory, buffer, check, message, opened } = new WebAssembly.Instance(compiled, {}).exports;
  // Each call may grow the memory, which replaces `memory.buffer`: read it
  // only after the call.
  return (query) => {
    const bytes = new TextEncoder().encode(query);
    const at = buffer(bytes.length);
    new Uint8Array(memory.buffer, at, bytes.length).set(bytes);
    const length = check();
    const text = new TextDecoder().decode(new Uint8Array(memory.buffer, message(), length));
    return opened() ? { problem: null, description: text } : { problem: text, description: null };
  };
}
