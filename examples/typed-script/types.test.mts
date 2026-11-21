import process, { ElevationError, isElevated, stdout } from "neo:process";
import { readFile, readFileSync, writeFile, writeFileSync, stat, statSync, lstat, lstatSync, readDir, readDirSync, mkdir, mkdirSync } from "neo:fs";
import type { DirEntry, FileInfo } from "neo:fs";

const elevated: boolean = isElevated();
const alsoElevated: boolean = process.isElevated();
const elevationError: typeof ElevationError = process.ElevationError;

// This is a compile-only fixture. An unused directive fails the check.
// @ts-expect-error isElevated is synchronous and returns a boolean.
const pendingElevation: Promise<boolean> = isElevated();

// @ts-expect-error stdout.write accepts strings or Uint8Arrays, not numbers.
stdout.write(42);

// @ts-expect-error os.version is optional and cannot always be used as a string.
os.version.toUpperCase();

const pending: Promise<Uint8Array> = readFile("data.bin");
const bytes: Uint8Array = readFileSync("data.bin");
const globalPending: Promise<Uint8Array> = fs.readFile("data.bin");
const globalBytes: Uint8Array = fs.readFileSync("data.bin");
const pendingWrite: Promise<void> = writeFile("data.bin", new Uint8Array());
const syncedWrite: void = writeFileSync("data.bin", new Uint8Array());
const globalPendingWrite: Promise<void> = fs.writeFile("data.bin", new Uint8Array());
const globalSyncedWrite: void = fs.writeFileSync("data.bin", new Uint8Array());

const pendingInfo: Promise<FileInfo> = stat("data.bin");
const syncedInfo: FileInfo = statSync("data.bin");
const pendingLink: Promise<FileInfo> = lstat("data.bin");
const syncedLink: FileInfo = lstatSync("data.bin");
const globalPendingInfo: Promise<FileInfo> = fs.stat("data.bin");
const globalSyncedInfo: FileInfo = fs.statSync("data.bin");
const globalPendingLink: Promise<FileInfo> = fs.lstat("data.bin");
const globalSyncedLink: FileInfo = fs.lstatSync("data.bin");

const pendingEntries: Promise<DirEntry[]> = readDir(".");
const syncedEntries: DirEntry[] = readDirSync(".");
const globalPendingEntries: Promise<DirEntry[]> = fs.readDir(".");
const globalSyncedEntries: DirEntry[] = fs.readDirSync(".");

const pendingMkdir: Promise<void> = mkdir("dir", { recursive: true });
const syncedMkdir: void = mkdirSync("dir", { recursive: true });
const globalPendingMkdir: Promise<void> = fs.mkdir("dir", { recursive: true });
const globalSyncedMkdir: void = fs.mkdirSync("dir", { recursive: true });

// @ts-expect-error The unsuffixed method no longer returns metadata synchronously.
const notMetadata: FileInfo = stat("data.bin");
// @ts-expect-error The unsuffixed global method returns metadata asynchronously.
const notGlobalMetadata: FileInfo = fs.stat("data.bin");
// @ts-expect-error The unsuffixed lstat no longer returns metadata synchronously.
const notLinkMetadata: FileInfo = lstat("data.bin");
// @ts-expect-error The unsuffixed readDir no longer returns entries synchronously.
const notEntries: DirEntry[] = readDir(".");
// @ts-expect-error The unsuffixed mkdir no longer completes synchronously.
const notMkdir: void = mkdir("dir");

// @ts-expect-error The unsuffixed method no longer returns bytes synchronously.
const notBytes: Uint8Array = readFile("data.bin");
// @ts-expect-error The unsuffixed write no longer completes synchronously.
const notSynchronous: void = writeFile("data.bin", new Uint8Array());
// @ts-expect-error The unsuffixed global write no longer completes synchronously.
const notGlobalSynchronous: void = fs.writeFile("data.bin", new Uint8Array());
