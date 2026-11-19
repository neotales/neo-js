/// <reference path="./neo-js.d.ts" />

import {
  basename,
  common,
  DELIMITER,
  dirname,
  extname,
  format,
  fromFileUrl,
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
  toFileUrl,
  toNamespacedPath,
  win32,
  type GlobOptions,
  type FileURL,
  type FileURLConstructor,
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
const url: FileURL = toFileUrl("/a/file.txt");
url.pathname = "/a/new%20file.txt";
const FileUrlClass = (globalThis as { URL?: FileURLConstructor }).URL;
if (FileUrlClass) fromFileUrl(new FileUrlClass("file:///a"));
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
  fromFileUrl(url),
  explicitPosix.fromFileUrl("file:///a"),
  win32.fromFileUrl(win32.toFileUrl("C:\\base")),
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
// @ts-expect-error Other path helpers do not accept URLs.
win32.toNamespacedPath(url);
// @ts-expect-error The runtime requires a branded instance, not a URL-shaped object.
fromFileUrl({ href: "file:///a" });
// @ts-expect-error This is a file-only URL subset without live search params.
url.searchParams;
// @ts-expect-error Explicit dialect namespaces are readonly.
win32.resolve = resolve;
// @ts-expect-error Compile flags are booleans.
globToRegExp("*.ts", { caseInsensitive: "yes" });
// @ts-expect-error joinGlobs accepts an array of strings.
joinGlobs("*.ts");
// @ts-expect-error match takes a path and glob, not a RegExp.
match("file.ts", /file/);
