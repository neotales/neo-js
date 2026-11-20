use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn run(script: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neojs"))
        .args(args)
        .arg(script)
        .output()
        .unwrap()
}

fn write(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, source).unwrap();
    path
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cli_runs_a_typescript_entry() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "interface Point { x: number; y: number }
type Pair = [number, number];
const point: Point = { x: 1, y: 2 };
const pair = [point.x, point.y] satisfies Pair;
class Counter {
  #count: number = 0;
  increment(by: number = 1): void { this.#count += by; }
  get value(): number { return this.#count; }
}
const counter = new Counter();
counter.increment();
console.log(`${pair.join(\",\")} ${counter.value}`);
",
    );
    let output = run(&main, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"1,2 1\n");
}

#[test]
fn cli_runs_nested_typescript_imports() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "import { scale } from './lib/math.ts';
console.log(`${scale(2)}`);
",
    );
    write(
        dir.path(),
        "lib/math.ts",
        "import { base } from '../shared/base.mts';
export type Factor = number;
export function scale(factor: Factor): number { return base * factor; }
",
    );
    write(
        dir.path(),
        "shared/base.mts",
        "export const base: number = 21;
export type Unused = string;
",
    );
    let output = run(&main, &["--allow-fs"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn type_only_imports_need_no_target_and_no_permission() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "import type { Shape } from './shapes.ts';
import { type Unused } from './also-missing.ts';
export type { Shape };
const value: number = 1;
console.log(`types ok ${value}`);
",
    );
    let output = run(&main, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"types ok 1\n");
}

#[test]
fn value_import_side_effects_survive_stripping() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "import { unused } from './effect.ts';
console.log('main');
",
    );
    write(
        dir.path(),
        "effect.ts",
        "console.log('side effect');
export const unused: number = 1;
",
    );
    let output = run(&main, &["--allow-fs"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"side effect\nmain\n");
}

#[test]
fn cli_runs_a_dynamically_imported_typescript_module() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "const loaded: typeof import('./dynamic.ts') = await import('./dynamic.ts');
console.log(`${loaded.value}`);
",
    );
    write(
        dir.path(),
        "dynamic.ts",
        "export const value: number = 41 + 1;
export type Ignored = string;
",
    );
    let output = run(&main, &["--allow-fs"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn typescript_imports_require_filesystem_permission() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "import { value } from './dependency.ts';
console.log(`${value}`);
",
    );
    write(
        dir.path(),
        "dependency.ts",
        "export const value: number = 7;\n",
    );

    let denied = run(&main, &[]);
    assert!(!denied.status.success());

    let allowed = run(&main, &["--allow-fs"]);
    assert!(allowed.status.success(), "{}", stderr(&allowed));
    assert_eq!(allowed.stdout, b"7\n");
}

#[test]
fn stripped_output_runs_on_quickjs() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "modern.ts",
        "type Entry = { name: string };
const entries: Entry[] = [{ name: 'a' }, { name: 'b' }];
const [first, ...rest] = entries;
const label = first?.name ?? 'none';
const count = rest?.length ?? 0;
const big = 1_000n;
class Box {
  static size: number = 1;
  #value: number;
  constructor(value: number) { this.#value = value; }
  get value(): number { return this.#value; }
}
console.log(`${label} ${count} ${big} ${Box.size} ${new Box(2 as number).value}`);
",
    );
    let output = run(&main, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"a 1 1000 1 2\n");
}

#[test]
fn type_only_import_conflicts_are_reported_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "conflict.ts",
        "import { type T } from './none.ts';
type T = number;
throw Error('executed');
",
    );
    // No --allow-fs: the type-only import must not be resolved, and the module
    // must not run far enough to reach the throw.
    let output = run(&main, &[]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("[TS_SEMANTIC]"), "{message}");
    assert!(message.contains("conflict.ts:1:15"), "{message}");
    assert!(!message.contains("executed"), "{message}");
}

#[test]
fn aliased_typescript_specifiers_are_one_module() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "import './a.ts';
import '././a.ts';
console.log('main');
",
    );
    write(
        dir.path(),
        "a.ts",
        "import './b.ts';
console.log('a');
",
    );
    write(
        dir.path(),
        "b.ts",
        "import '././a.ts';
console.log('b');
",
    );
    let output = run(&main, &["--allow-fs"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"b\na\nmain\n");
}

#[test]
fn an_entry_alias_runs_the_entry_once() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "self.ts",
        "import '././self.ts';
console.log('entry');
",
    );
    let output = run(&main, &["--allow-fs"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"entry\n");
}

#[test]
fn aliased_javascript_specifiers_are_one_module() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("lib")).unwrap();
    let main = write(
        dir.path(),
        "main.js",
        "import './dep.js';
import '././dep.js';
import './lib/../dep.js';
console.log('main');
",
    );
    write(dir.path(), "dep.js", "console.log('dep');\n");
    let output = run(&main, &["--allow-fs"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"dep\nmain\n");
}

#[test]
fn parser_and_semantic_errors_are_reported_before_execution() {
    let dir = tempfile::tempdir().unwrap();

    let syntax = write(
        dir.path(),
        "syntax.ts",
        "const ok = 1;\nconst bad: number = ;\n",
    );
    let output = run(&syntax, &[]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("syntax.ts:2:"), "{message}");
    assert!(message.contains("[TS_PARSE]"), "{message}");

    let semantic = write(
        dir.path(),
        "semantic.ts",
        "let value = 1;\nlet value = 2;\n",
    );
    let output = run(&semantic, &[]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("semantic.ts:2:"), "{message}");
    assert!(message.contains("[TS_SEMANTIC]"), "{message}");
}

#[test]
fn non_erasable_constructs_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("enum", "export enum Colour { Red }\n", "enum"),
        (
            "parameter-property",
            "export class Point { constructor(public x: number) {} }\n",
            "parameter property",
        ),
        (
            "namespace",
            "namespace Util { export const one = 1; }\n",
            "namespace",
        ),
        (
            "decorator",
            "export class Marked { @sealed method() {} }\n",
            "decorator",
        ),
        (
            "import-equals",
            "import fs = require('./fs.ts');\n",
            "import =",
        ),
        (
            "export-assignment",
            "const value = 1;\nexport = value;\n",
            "export =",
        ),
    ];
    for (file, source, construct) in cases {
        let script = write(dir.path(), &format!("{file}.ts"), source);
        let output = run(&script, &[]);
        assert!(!output.status.success(), "{file} should fail");
        let message = stderr(&output);
        assert!(message.contains("[TS_UNSUPPORTED]"), "{file}: {message}");
        assert!(message.contains(construct), "{file}: {message}");
        assert!(
            message.contains(&format!("{file}.ts:")),
            "{file}: {message}"
        );
    }
}

#[test]
fn a_nested_typescript_error_names_the_imported_file() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "main.ts",
        "import { value } from './broken.ts';
console.log(`${value}`);
",
    );
    write(
        dir.path(),
        "broken.ts",
        "export const value: number = 1;\nexport enum Colour { Red }\n",
    );
    let output = run(&main, &["--allow-fs"]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("broken.ts:2:8"), "{message}");
    assert!(message.contains("[TS_UNSUPPORTED]"), "{message}");
}

#[test]
fn declaration_and_unsupported_extensions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    for (file, construct) in [
        ("types.d.ts", "[TS_EXTENSION]"),
        ("common.cts", "[TS_EXTENSION]"),
        ("view.tsx", "[TS_EXTENSION]"),
    ] {
        let script = write(dir.path(), file, "export const value: number = 1;\n");
        let output = run(&script, &[]);
        assert!(!output.status.success(), "{file} should fail");
        let message = stderr(&output);
        assert!(message.contains(construct), "{file}: {message}");
    }
}

#[test]
fn javascript_is_still_evaluated_without_stripping() {
    let dir = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "typed.js", "const value: number = 1;\n");
    let output = run(&script, &[]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(!message.contains("TS_"), "{message}");
}

#[test]
fn default_and_namespace_type_import_conflicts_have_no_side_effects() {
    for import in [
        "import type T from './missing.ts';",
        "import type * as T from './missing.ts';",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "conflict.ts",
            &format!("{import}\ntype T = number;\nconsole.log('executed');"),
        );
        let output = run(&main, &[]);
        assert!(!output.status.success(), "{import}");
        assert!(
            stderr(&output).contains("[TS_SEMANTIC]"),
            "{import}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty());
    }
}
