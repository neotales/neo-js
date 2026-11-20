/// <reference path="./neo-js.d.ts" />

import { type Database, openMemory, type SqliteError, type SqliteValue } from "neo:sqlite";

using db: Database = openMemory();
const changed: bigint = db.execute("CREATE TABLE items(name TEXT)");
const rows: SqliteValue[][] = db.query("SELECT name FROM items WHERE name = ?", ["name"]);

// These directives are load-bearing. If the checker treats a missing
// `neo:sqlite` as `any`, each `@ts-expect-error` becomes unused and fails.
// @ts-expect-error openMemory takes no arguments.
openMemory("file");
// @ts-expect-error execute requires a string SQL statement.
db.execute(1);
// @ts-expect-error Database has no run method.
db.run("SELECT 1");
// @ts-expect-error SqliteValue does not include boolean.
const notValue: SqliteValue = true;

db.close();

function sqliteErrorCode(error: SqliteError): string {
  return error.code;
}

// deno check only; the fixture runs under neo-js, not Deno.
void [changed, rows, notValue, sqliteErrorCode];
