// Optional synchronous in-memory byte reader and writer for neojs-io.
// Referenced by the compose-types selection; core declarations deliberately
// omit this module.

/**
 * Synchronous byte readers and writers over bounded in-memory storage.
 *
 * This module needs no permission flag and grants no host capability. It never
 * reads or writes a file, directory, socket, process, environment variable, or
 * registry key. It is not a Node.js stream, an `EventEmitter`, or a web stream:
 * there are no events, listeners, promises, `pipe`, `on`, backpressure
 * signalling, encodings, or `Buffer` values.
 *
 * `byteReader` copies the current view of its `Uint8Array` into reader-owned
 * storage. Mutating, detaching, or resizing the caller's buffer afterwards does
 * not change the reader. `byteWriter` owns a growable buffer; `toUint8Array`
 * returns a fresh copy that the caller owns, and changing that copy does not
 * change the writer. `close` releases the stored bytes, is idempotent, and
 * never detaches a caller's buffer.
 *
 * Both objects are synchronous and single-threaded. Inputs are checked through
 * the engine's internal typed-array brand rather than through mutable globals
 * or prototype properties. A detached, shared, resizable, forged, or proxied
 * value is rejected with the matching `IoError.code`. A limit violation or
 * allocation failure is reported before any state changes, so a failed `write`
 * never partially appends.
 *
 * `MAX_RUNTIME_BYTES` bounds native bytes across all live readers and writers
 * in one runtime; `MAX_OBJECTS` bounds the live object count. A closed object
 * releases its bytes but keeps its slot until it is collected or the runtime
 * is dropped. A `Uint8Array` returned by `toUint8Array` is JavaScript-owned and
 * outside this native budget after return.
 *
 * Calling a method or getter with a receiver that is not an object created by
 * this module throws the engine's native `TypeError`, not an `IoError`.
 *
 * @example Reading a bounded snapshot
 * ```ts
 * import { byteReader } from "neo:io";
 *
 * const reader = byteReader(Uint8Array.from([1, 2, 3]));
 * const target = new Uint8Array(2);
 * reader.read(target);        // 2
 * reader.read(target);        // 1
 * reader.read(target);        // 0, end of file
 * reader.close();
 * ```
 *
 * @example Writing owned bytes
 * ```ts
 * import { byteWriter } from "neo:io";
 *
 * const writer = byteWriter();
 * writer.write(new Uint8Array([1, 2, 3]));
 * const bytes = writer.toUint8Array(); // caller-owned copy
 * writer.close();
 * ```
 */
declare module "neo:io" {
  /** Stable failure codes; match on these, never on the message. */
  export type IoErrorCode =
    | "IO_ARGUMENT"
    | "IO_LIMIT"
    | "IO_CLOSED"
    | "IO_DETACHED"
    | "IO_SHARED"
    | "IO_RESIZABLE";

  /**
   * The native `Error` shape thrown by this module. The value is an ordinary
   * `Error` from the current realm with `name = "IoError"` and an own `code`
   * property; there is no exported constructor to test with `instanceof`.
   * Messages are static and never include the rejected bytes.
   */
  export interface IoError extends Error {
    name: "IoError";
    /** Stable code; match on this, never on the message. */
    code: IoErrorCode;
  }

  /**
   * A synchronous reader over an owned snapshot of a `Uint8Array`.
   *
   * `read` copies at most `target.byteLength` bytes and returns the number
   * copied. It returns exactly `0` at end of file; end of file is not an
   * exception. An empty target throws `IO_ARGUMENT` rather than returning an
   * ambiguous zero. A closed reader reports `remaining = 0`, keeps
   * `closed = true`, and throws `IO_CLOSED` before validating `target`.
   */
  export interface ByteReader {
    /** Bytes not yet read; `0` after end of file or after `close`. */
    readonly remaining: number;
    /** Whether `close` has been called. */
    readonly closed: boolean;
    /**
     * Copy bytes into `target`.
     *
     * @param target An attached, fixed-length `Uint8Array` with at least one
     *   byte. It may be larger than the remaining input.
     * @returns The number of bytes copied, or `0` at end of file.
     * @throws IoError `IO_ARGUMENT`, `IO_CLOSED`, `IO_DETACHED`, `IO_SHARED`,
     *   or `IO_RESIZABLE`.
     */
    read(target: Uint8Array): number;
    /** Release the owned bytes. Idempotent; further reads throw `IO_CLOSED`. */
    close(): void;
  }

  /**
   * A synchronous writer over an owned, bounded growable byte buffer.
   *
   * `write` appends the whole input or throws `IO_LIMIT` without changing the
   * writer. `toUint8Array` returns a fresh copy; it never exposes the internal
   * buffer. A closed writer reports `length = 0`, keeps `closed = true`, and
   * both `write` and `toUint8Array` throw `IO_CLOSED` before validating input.
   * A closed object keeps its object slot until it is collected or the
   * runtime is dropped.
   */
  export interface ByteWriter {
    /** Bytes currently held; `0` after `close`. */
    readonly length: number;
    /** Whether `close` has been called. */
    readonly closed: boolean;
    /**
     * Append all of `bytes`.
     *
     * @param bytes An attached, fixed-length `Uint8Array`. A zero-length input
     *   is accepted and returns `0`.
     * @returns The number of bytes appended, equal to `bytes.byteLength`.
     * @throws IoError `IO_ARGUMENT`, `IO_LIMIT`, `IO_CLOSED`, `IO_DETACHED`,
     *   `IO_SHARED`, or `IO_RESIZABLE`.
     */
    write(bytes: Uint8Array): number;
    /**
     * Return a fresh `Uint8Array` copy of the current bytes. The caller owns
     * the result; mutating it does not change the writer.
     *
     * @throws IoError `IO_CLOSED`, or `IO_LIMIT` when the temporary copy would
     *   exceed the runtime aggregate.
     */
    toUint8Array(): Uint8Array;
    /** Release the owned bytes. Idempotent; further writes throw `IO_CLOSED`. */
    close(): void;
  }

  /**
   * Create a reader from an owned snapshot of `bytes`.
   *
   * The input view is copied at construction, so later mutation, detaching, or
   * resizing of the caller's buffer does not change the reader.
   *
   * @throws IoError `IO_ARGUMENT`, `IO_LIMIT`, `IO_DETACHED`, `IO_SHARED`, or
   *   `IO_RESIZABLE`.
   */
  export function byteReader(bytes: Uint8Array): ByteReader;

  /**
   * Create an empty writer.
   *
   * @throws IoError `IO_LIMIT` when the runtime object limit is reached.
   */
  export function byteWriter(): ByteWriter;

  /** Maximum bytes in one reader or writer, and in one `write` call. */
  export const MAX_BYTES: 8388608;

  /** Maximum native bytes across every live reader and writer in one runtime. */
  export const MAX_RUNTIME_BYTES: 16777216;

  /** Maximum number of live reader and writer objects in one runtime. */
  export const MAX_OBJECTS: 32;
}
