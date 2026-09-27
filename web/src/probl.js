// Probl's WebAssembly module (crates/probl-wasm), called with text.
//
// Works in browsers, in workers and in Node. The module's functions take and
// give JSON: `check` gives a program's diagnostics, `run` runs it, and
// `examples` gives the bundled examples.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** The module stopped: it panicked, overflowed its stack, or ran out of
 * memory. It can't be used again; load a new one. */
export class Crash extends Error {}

/**
 * Instantiate the module, from its bytes or a compiled `WebAssembly.Module`.
 * `onPrint` receives each line a program prints, as it prints it, and
 * `onProgress`, when sampling, the runs done and the runs in all, after each
 * batch.
 */
export async function load(module, { onPrint = () => {}, onProgress = () => {} } = {}) {
  let memory;
  let panic = null;
  const text = (ptr, len) => decoder.decode(new Uint8Array(memory.buffer, ptr, len));
  const imports = {
    probl: {
      print: (ptr, len) => onPrint(text(ptr, len)),
      panicked: (ptr, len) => {
        panic = text(ptr, len);
      },
      progress: (done, total) => onProgress(done, total),
    },
  };
  const made = await WebAssembly.instantiate(module, imports);
  const exports = (made.instance ?? made).exports;
  memory = exports.memory;

  function call(fn, input) {
    const args = [];
    if (input !== undefined) {
      const bytes = encoder.encode(input);
      const ptr = exports.probl_alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
      args.push(ptr, bytes.length);
    }
    let len;
    try {
      len = fn(...args);
    } catch (error) {
      throw new Crash(panic ?? String(error));
    }
    return text(exports.probl_result(), len);
  }

  return {
    /** `{diagnostics: [...]}` for a program's source. */
    check: (source) => JSON.parse(call(exports.probl_check, source)),
    /** Run `{source, mode?, runs?, seed?, conjugate?, files?}`. */
    run: (request) => JSON.parse(call(exports.probl_run, JSON.stringify(request))),
    /** `[{name, title, source, files}]` */
    examples: () => JSON.parse(call(exports.probl_examples)),
    version: () => call(exports.probl_version),
    /** Bytes of memory the module holds, which only grows. */
    memoryBytes: () => memory.buffer.byteLength,
  };
}
