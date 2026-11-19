/// <reference path="./neo-js.d.ts" />

import { type Database, openMemory, type SqliteError, type SqliteValue } from "neo:sqlite";

using db: Database = openMemory();
const changed: bigint = db.execute("CREATE TABLE items(name TEXT)");
const rows: SqliteValue[][] = db.query("SELECT name FROM items WHERE name = ?", ["name"]);
db.close();

function sqliteErrorCode(error: SqliteError): string {
  return error.code;
}

// deno check only; the fixture runs under neo-js, not Deno.
void [changed, rows, sqliteErrorCode];
