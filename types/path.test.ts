/// <reference path="./neo-js.d.ts" />

import {
  basename,
  common,
  DELIMITER,
  dirname,
  extname,
  format,
  globToRegExp,
  isAbsolute,
  isGlob,
  join,
  joinGlobs,
  match,
  matchesGlob,
  normalize,
  normalizeGlob,
  parse,
  posix,
  relative,
  resolve,
  SEPARATOR,
  SEPARATOR_PATTERN,
  toNamespacedPath,
  win32,
  type GlobOptions,
  type ParsedPath,
  type PathApi,
} from "neo:path";

const separator: "/" | "\\" = SEPARATOR;
const delimiter: ":" | ";" = DELIMITER;
const pattern: RegExp = SEPARATOR_PATTERN;
const explicitPosix: PathApi<"/", ":"> = posix;
const winSeparator: "\\" = win32.sep;
const parsed: ParsedPath = parse("/a/file.txt");
const globOptions: GlobOptions = { extended: true, globstar: true, caseInsensitive: true };
const globPattern: RegExp = globToRegExp("**/*.txt", globOptions);
const paths: string[] = [
  join("a", "b"),
  normalize("a/.."),
  basename("/a/file.txt", ".txt"),
  dirname("/a/file.txt"),
  extname("/a/file.txt"),
  format(parsed),
  common(["/a/b", "/a/c"] as const),
  resolve("/a", "b"),
  relative("/a/b", "/a/c"),
  toNamespacedPath("/a/b"),
  explicitPosix.resolve("/a", "b"),
  win32.relative("C:\\base", "C:\\base\\file"),
  joinGlobs(["a", "**", ".."] as const, globOptions),
  normalizeGlob("**/..", { globstar: true }),
];
const absolute: boolean = isAbsolute(separator);
"a///b".split(pattern);
void [delimiter, winSeparator, paths, absolute];
const globResults: boolean[] = [
  isGlob("*.txt"),
  match("a/file.txt", "**/*.txt"),
  matchesGlob("a/file.txt", "**/*.txt"),
  win32.matchesGlob("a\\file.txt", "**/*.txt"),
  explicitPosix.globToRegExp("**/*.txt").test("a/file.txt"),
  globPattern.test("FILE.TXT"),
];
void globResults;

// @ts-expect-error A common-prefix input is an array of strings.
common("a");
// @ts-expect-error Resolve requires string segments.
resolve("/a", 1);
// @ts-expect-error Relative requires two operands.
relative("/a");
// @ts-expect-error No URL inputs.
win32.toNamespacedPath(new URL("file:///a"));
// @ts-expect-error Explicit dialect namespaces are readonly.
win32.resolve = resolve;
// @ts-expect-error Compile flags are booleans.
globToRegExp("*.ts", { caseInsensitive: "yes" });
// @ts-expect-error joinGlobs accepts an array of strings.
joinGlobs("*.ts");
// @ts-expect-error match takes a path and glob, not a RegExp.
match("file.ts", /file/);
