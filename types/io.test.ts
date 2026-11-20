import { MAX_BYTES, MAX_OBJECTS, MAX_RUNTIME_BYTES, byteReader, byteWriter } from "neo:io";
import type { ByteReader, ByteWriter, IoError, IoErrorCode } from "neo:io";

const reader: ByteReader = byteReader(new Uint8Array([1, 2, 3]));
const target = new Uint8Array(2);
const copied: number = reader.read(target);
const remaining: number = reader.remaining;
const readerClosed: boolean = reader.closed;
reader.close();

const writer: ByteWriter = byteWriter();
const written: number = writer.write(target);
const bytes: Uint8Array = writer.toUint8Array();
const length: number = writer.length;
const writerClosed: boolean = writer.closed;
writer.close();

const limit: 8388608 = MAX_BYTES;
const runtimeLimit: 16777216 = MAX_RUNTIME_BYTES;
const objectLimit: 32 = MAX_OBJECTS;

const code: IoErrorCode = "IO_CLOSED";
try {
  reader.read(target);
} catch (error) {
  if (error instanceof Error && error.name === "IoError") {
    const ioError = error as IoError;
    const actual: IoErrorCode = ioError.code;
    void actual;
  }
}

void copied;
void remaining;
void readerClosed;
void written;
void bytes;
void length;
void writerClosed;
void limit;
void runtimeLimit;
void objectLimit;
void code;

// @ts-expect-error a reader needs its own Uint8Array
byteReader("bytes");

// @ts-expect-error this is not a Node stream
reader.on("data", () => {});

// @ts-expect-error this is not a Node stream
writer.pipe(reader);

// @ts-expect-error the module has no Buffer surface
const buffer: Buffer = writer.toUint8Array();

// @ts-expect-error MAX_BYTES is the literal 8388608
const notTheLimit: 1 = MAX_BYTES;
void [buffer, notTheLimit];
