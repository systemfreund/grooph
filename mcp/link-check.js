// grooph-link's parser as a WebAssembly module (crates/link-check), so the
// tool checks its link by the rules the app opens it with. The module has no
// imports and three exports: `buffer(len)` for the query, `check()` returning
// the length of the message (0: the link opens) and `message()` where it is.

/** `checkLink(query)`: why the link with this query (without `?`) would not
 * open in the app, or `null`. `module` is the compiled module or its bytes. */
export function linkChecker(module) {
  const compiled = module instanceof WebAssembly.Module ? module : new WebAssembly.Module(module);
  const { memory, buffer, check, message } = new WebAssembly.Instance(compiled, {}).exports;
  // Each call may grow the memory, which replaces `memory.buffer`: read it
  // only after the call.
  return (query) => {
    const bytes = new TextEncoder().encode(query);
    const at = buffer(bytes.length);
    new Uint8Array(memory.buffer, at, bytes.length).set(bytes);
    const length = check();
    if (!length) return null;
    const from = message();
    return new TextDecoder().decode(new Uint8Array(memory.buffer, from, length));
  };
}
