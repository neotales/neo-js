//! Integration tests for `neo:semver`.
//!
//! These use the public `NeoJs` API and check the documented JavaScript
//! contract: exact component precision, precedence, ranges, errors, private
//! callbacks, and use from a stripped TypeScript entry.

use neo_js::{ModuleAllowlist, NeoJs, Permissions, RuntimeModule, RuntimeOptions};

fn runtime() -> NeoJs {
    NeoJs::new(Permissions::default()).unwrap()
}

fn run(js: &NeoJs, source: &str) {
    js.run_module("main.js", source).unwrap();
}

#[test]
fn official_precedence_vectors_hold_and_build_is_ignored() {
    run(
        &runtime(),
        r#"
        import { compare } from "neo:semver";
        const ordered = [
          "1.0.0-alpha",
          "1.0.0-alpha.1",
          "1.0.0-alpha.beta",
          "1.0.0-beta",
          "1.0.0-beta.2",
          "1.0.0-beta.11",
          "1.0.0-rc.1",
          "1.0.0",
        ];
        for (let index = 1; index < ordered.length; index++) {
          if (compare(ordered[index - 1], ordered[index]) !== -1) {
            throw Error("not ascending at " + ordered[index]);
          }
          if (compare(ordered[index], ordered[index - 1]) !== 1) {
            throw Error("not descending at " + ordered[index]);
          }
        }
        if (compare("1.0.0+one", "1.0.0+two") !== 0) throw Error("build changed precedence");
        if (compare("1.0.0", "1.0.0") !== 0) throw Error("identity");
        if (compare("1.0.0-9007199254740992", "1.0.0-9007199254740993") !== -1) {
          throw Error("unsafe integer prerelease order");
        }
        if (compare(
          "1.0.0-100000000000000000000",
          "1.0.0-99999999999999999999"
        ) !== 1) throw Error("arbitrary precision prerelease order");
        "#,
    );
}

#[test]
fn parse_returns_exact_string_components_and_null_prototypes() {
    run(
        &runtime(),
        r#"
        import { parse, format } from "neo:semver";
        const source = "18446744073709551615.0.1-alpha.9007199254740993+sha.001";
        const value = parse(source);
        if (Object.getPrototypeOf(value) !== null) throw Error("version prototype");
        if (value.version !== source) throw Error("canonical version");
        if (value.major !== "18446744073709551615") throw Error("major precision");
        if (value.minor !== "0" || value.patch !== "1") throw Error("core");
        if (value.prerelease[0] !== "alpha") throw Error("prerelease");
        if (value.prerelease[1] !== "9007199254740993") throw Error("prerelease precision");
        if (value.build[0] !== "sha" || value.build[1] !== "001") throw Error("build");
        if (!Array.isArray(value.prerelease) || !Array.isArray(value.build)) {
          throw Error("identifier arrays");
        }
        const own = Object.prototype.hasOwnProperty;
        for (const key of ["version", "major", "minor", "patch", "prerelease", "build"]) {
          if (!own.call(value, key)) throw Error("missing own " + key);
        }
        if (!own.call(value.prerelease, "1")) throw Error("prerelease index is not own");
        if (format(value) !== source) throw Error("format(parse(x))");
        if (format({
          major: "1",
          minor: 2,
          patch: "3",
          prerelease: ["beta", "1"],
          build: ["sha", "001"],
        }) !== "1.2.3-beta.1+sha.001") throw Error("format components");
        "#,
    );
}

#[test]
fn invalid_and_partial_versions_are_rejected_with_stable_codes() {
    run(
        &runtime(),
        r#"
        import { valid, compare, parse, SemVerError } from "neo:semver";
        const invalid = [
          "01.2.3", "1.02.3", "1.2.03", "1.2", "1", "", " 1.2.3", "1.2.3 ",
          "1.2.3-01", "1.2.3-alpha.01", "1.2.3-alpha..1", "1.2.3+", "1.2.3-", "v1.2.3",
          "1.2.3-alpha_1", "1.2.3+build+two",
        ];
        for (const value of invalid) {
          if (valid(value)) throw Error("accepted invalid " + value);
          let error;
          try { parse(value); } catch (caught) { error = caught; }
          if (!(error instanceof SemVerError)) throw Error("wrong error type for " + value);
          if (error.name !== "SemVerError") throw Error("wrong name");
          if (error.code !== "SEMVER_INVALID") throw Error("wrong code for " + value);
        }
        if (!valid("0.0.0")) throw Error("zero version rejected");
        if (!valid("1.2.3-alpha.0+build.01")) throw Error("valid version rejected");
        let code = "";
        try { compare("1.2", "1.2.3"); } catch (error) { code = error.code; }
        if (code !== "SEMVER_INVALID") throw Error("compare invalid code");
        "#,
    );
}

#[test]
fn arguments_are_not_coerced() {
    run(
        &runtime(),
        r#"
        import { valid, compare, parse, format, increment, satisfies, validRange } from "neo:semver";
        const calls = [
          () => valid(1),
          () => valid(new String("1.2.3")),
          () => compare("1.2.3", null),
          () => parse({}),
          () => format(null),
          () => format({ major: "1", minor: "2", patch: "3", prerelease: "beta" }),
          () => increment("1.2.3", "patch", 5),
          () => satisfies("1.2.3", undefined),
          () => validRange(1),
        ];
        for (const call of calls) {
          let error;
          try { call(); } catch (caught) { error = caught; }
          if (!(error instanceof TypeError)) throw Error("argument was coerced");
        }
        "#,
    );
}

#[test]
fn oversized_core_components_are_rejected_instead_of_rounded() {
    run(
        &runtime(),
        r#"
        import { parse } from "neo:semver";
        let error;
        try { parse("18446744073709551616.0.0"); } catch (caught) { error = caught; }
        if (!error || error.code !== "SEMVER_INVALID") throw Error("wrong code");
        if (typeof error.message !== "string" || error.message.length === 0) {
          throw Error("missing explanation");
        }
        "#,
    );
}

#[test]
fn formatting_and_incrementing_are_documented_and_non_destructive() {
    run(
        &runtime(),
        r#"
        import { format, increment } from "neo:semver";
        if (increment("1.2.3+build", "major") !== "2.0.0") throw Error("major");
        if (increment("1.2.3", "minor") !== "1.3.0") throw Error("minor");
        if (increment("1.2.3", "patch") !== "1.2.4") throw Error("patch");
        if (increment("1.2.3", "premajor", "beta") !== "2.0.0-beta.0") throw Error("premajor");
        if (increment("1.2.3", "preminor") !== "1.3.0-0") throw Error("preminor");
        if (increment("1.2.3", "prepatch", "rc") !== "1.2.4-rc.0") throw Error("prepatch");
        if (increment("1.0.0-alpha", "major") !== "2.0.0") throw Error("prerelease major");
        if (increment("1.2.3-alpha.9", "prerelease") !== "1.2.3-alpha.10") {
          throw Error("prerelease counter");
        }
        if (increment("1.2.3-alpha.9", "prerelease", "beta") !== "1.2.3-beta.0") {
          throw Error("prerelease restart");
        }
        let code = "";
        try { increment("1.2.3", "banana"); } catch (error) { code = error.code; }
        if (code !== "SEMVER_INCREMENT") throw Error("increment code");
        code = "";
        try { increment("18446744073709551615.0.0", "major"); } catch (error) { code = error.code; }
        if (code !== "SEMVER_OVERFLOW") throw Error("overflow code");
        code = "";
        try {
          format({ major: "01", minor: "2", patch: "3" });
        } catch (error) { code = error.code; }
        if (code !== "SEMVER_INVALID") throw Error("format code");
        "#,
    );
}

#[test]
fn ranges_use_cargo_semantics_and_document_prerelease_rules() {
    run(
        &runtime(),
        r#"
        import { satisfies, validRange } from "neo:semver";
        const yes = [
          ["1.2.3", "^1.2.0"],
          ["1.9.9", "^1.2.0"],
          ["1.9.0", "1.2"],
          ["1.9.0", "1.2.3"],
          ["0.2.9", "0.2"],
          ["0.9.9", "0"],
          ["1.3.0", ">=1.2.3, <2.0.0"],
          ["1.2.9", "1.2"],
          ["1.2.9", "1.2.*"],
          ["0.0.3", "^0.0.3"],
          ["0.0.9", "^0.0"],
          ["1.3.0-beta.2", ">=1.3.0-beta.1, <1.4.0"],
        ];
        for (const [version, range] of yes) {
          if (!satisfies(version, range)) throw Error(version + " should satisfy " + range);
        }
        const no = [
          ["2.0.0", "^1.2.0"],
          ["1.1.9", "1.2"],
          ["1.2.2", "1.2.3"],
          ["0.3.0", "0.2"],
          ["1.0.0", "0"],
          ["1.3.0-alpha", "^1.2.0"],
          ["1.4.0-alpha", ">=1.3.0-alpha, <1.4.0"],
          ["1.0.0-alpha", "*"],
          ["0.0.4", "^0.0.3"],
          ["0.1.0", "^0.0"],
        ];
        for (const [version, range] of no) {
          if (satisfies(version, range)) throw Error(version + " should not satisfy " + range);
        }
        for (const range of ["^1 || ^2", "1.2.3 - 2.0.0", ">=1.2.3 <2.0.0", "1.2.3,"]) {
          if (validRange(range)) throw Error("accepted npm-only range " + range);
          let code = "";
          try { satisfies("1.2.3", range); } catch (error) { code = error.code; }
          if (code !== "SEMVER_RANGE") throw Error("range code for " + range);
        }
        if (!validRange("^1.2.3")) throw Error("valid range rejected");
        "#,
    );
}

#[test]
fn input_and_format_limits_are_enforced_at_the_bound() {
    run(
        &runtime(),
        r#"
        import {
          valid, parse, compare, format, satisfies, validRange,
          MAX_VERSION_BYTES, MAX_RANGE_BYTES, MAX_IDENTIFIERS, MAX_IDENTIFIER_BYTES,
        } from "neo:semver";

        function repeat(count, text) {
          let value = "";
          for (let index = 0; index < count; index++) value += text;
          return value;
        }
        function identifiers(count, text) {
          let value = "";
          for (let index = 0; index < count; index++) {
            if (index > 0) value += ".";
            value += text;
          }
          return value;
        }
        function items(count, text) {
          const value = [];
          for (let index = 0; index < count; index++) value[index] = text;
          return value;
        }
        function codeOf(call) {
          try { call(); } catch (error) { return error.code; }
          return "";
        }

        // Exactly MAX_VERSION_BYTES: 32 prerelease identifiers of 30 bytes and
        // one 26-byte build identifier.
        let atVersion = "1.0.0-" + identifiers(MAX_IDENTIFIERS, repeat(30, "a"));
        atVersion += "+" + repeat(26, "b");
        if (atVersion.length !== MAX_VERSION_BYTES) throw Error("fixture version length");
        const parsed = parse(atVersion);
        if (parsed.prerelease.length !== MAX_IDENTIFIERS || parsed.build.length !== 1) {
          throw Error("fixture components");
        }
        if (format(parsed) !== atVersion) throw Error("format(parse(at-limit))");
        if (!valid(atVersion)) throw Error("at-limit version rejected");
        if (valid(atVersion + "b")) throw Error("over-limit version accepted");
        if (codeOf(() => parse(atVersion + "b")) !== "SEMVER_LIMIT") {
          throw Error("version limit code");
        }
        if (codeOf(() => compare(atVersion + "b", "1.0.0")) !== "SEMVER_LIMIT") {
          throw Error("compare should reject the over-limit operand");
        }
        if (compare(atVersion, atVersion) !== 0) throw Error("at-limit compare");

        // Exactly MAX_RANGE_BYTES with one comparator.
        const atRange = ">=1.0.0-" + repeat(MAX_RANGE_BYTES - 8, "a");
        if (atRange.length !== MAX_RANGE_BYTES) throw Error("fixture range length");
        if (!validRange(atRange)) throw Error("at-limit range rejected");
        if (validRange(atRange + "a")) throw Error("over-limit range accepted");
        if (codeOf(() => satisfies("1.0.0", atRange + "a")) !== "SEMVER_LIMIT") {
          throw Error("range limit code");
        }

        // Identifier count and per-identifier size.
        const countAt = "1.0.0-" + identifiers(MAX_IDENTIFIERS, "a");
        const countOver = "1.0.0-" + identifiers(MAX_IDENTIFIERS + 1, "a");
        if (!valid(countAt)) throw Error("count-at rejected");
        if (valid(countOver)) throw Error("count-over accepted");
        if (codeOf(() => parse(countOver)) !== "SEMVER_LIMIT") throw Error("count code");
        const sizeAt = "1.0.0-" + repeat(MAX_IDENTIFIER_BYTES, "a");
        const sizeOver = "1.0.0-" + repeat(MAX_IDENTIFIER_BYTES + 1, "a");
        if (!valid(sizeAt)) throw Error("size-at rejected");
        if (valid(sizeOver)) throw Error("size-over accepted");
        if (codeOf(() => parse(sizeOver)) !== "SEMVER_LIMIT") throw Error("size code");

        // Format must bound both arrays before the native copy and bound the
        // rendered total even when individual entries are legal.
        if (format({
          major: "1", minor: "0", patch: "0",
          prerelease: items(MAX_IDENTIFIERS, "a"),
        }) !== "1.0.0-" + identifiers(MAX_IDENTIFIERS, "a")) {
          throw Error("format at count limit");
        }
        if (codeOf(() => format({
          major: "1", minor: "0", patch: "0",
          prerelease: items(MAX_IDENTIFIERS + 1, "a"),
        })) !== "SEMVER_LIMIT") throw Error("format count code");
        if (codeOf(() => format({
          major: "1", minor: "0", patch: "0",
          prerelease: [repeat(MAX_IDENTIFIER_BYTES + 1, "a")],
        })) !== "SEMVER_LIMIT") throw Error("format size code");
        if (codeOf(() => format({
          major: "1", minor: "0", patch: "0",
          prerelease: items(MAX_IDENTIFIERS, repeat(100, "a")),
        })) !== "SEMVER_LIMIT") throw Error("format aggregate code");
        if (codeOf(() => format({
          major: repeat(21, "1"), minor: "0", patch: "0",
        })) !== "SEMVER_LIMIT") throw Error("format core code");
        const formatAt = format({
          major: "1", minor: "0", patch: "0",
          prerelease: items(MAX_IDENTIFIERS, repeat(30, "a")),
          build: items(1, repeat(26, "b")),
        });
        if (formatAt !== atVersion) throw Error("format at total limit");
        if (codeOf(() => format({
          major: "1", minor: "0", patch: "0",
          prerelease: items(MAX_IDENTIFIERS, repeat(30, "a")),
          build: items(1, repeat(27, "b")),
        })) !== "SEMVER_LIMIT") throw Error("format over total limit");
        "#,
    );
}

#[test]
fn increment_and_proxy_array_boundaries() {
    run(
        &runtime(),
        r#"
        import { increment, parse, valid, format, MAX_IDENTIFIERS, MAX_IDENTIFIER_BYTES, MAX_VERSION_BYTES } from "neo:semver";
        function code(fn) { try { fn(); } catch (e) { return e.code || e.name; } return ""; }
        let coercions = 0, reads = 0;
        const length = { valueOf() { coercions++; return coercions === 1 ? 0 : 1e9; } };
        const proxy = new Proxy([], { get(target, key) {
          if (key === "length") { reads++; return length; }
          throw Error("unexpected element read");
        } });
        if (code(() => format({major: 1, minor: 0, patch: 0, prerelease: proxy})) !== "TypeError") throw Error("exotic length");
        if (coercions !== 0 || reads !== 1) throw Error("coerced or reread length");
        for (const bad of [NaN, Infinity, -1, 1.5, "0"]) {
          const p = new Proxy([], { get(t, k) { if (k === "length") return bad; throw Error("element"); } });
          if (code(() => format({major: 1, minor: 0, patch: 0, build: p})) !== "TypeError") throw Error("bad length");
        }
        const over = new Proxy([], { get(t, k) { if (k === "length") return MAX_IDENTIFIERS + 1; throw Error("element"); } });
        if (code(() => format({major: 1, minor: 0, patch: 0, build: over})) !== "SEMVER_LIMIT") throw Error("proxy limit");
        if (code(() => increment("1.0.0", "x".repeat(100000))) !== "SEMVER_INCREMENT") throw Error("release limit");
        const at = "1.0.0-" + Array(MAX_IDENTIFIERS).fill("a").join(".");
        if (code(() => increment(at, "prerelease")) !== "SEMVER_LIMIT") throw Error("increment count");
        const below = "1.0.0-" + Array(MAX_IDENTIFIERS - 1).fill("a").join(".");
        const countOutput = increment(below, "prerelease");
        if (!valid(countOutput) || parse(countOutput).prerelease.length !== MAX_IDENTIFIERS) throw Error("count output");
        const sizeOutput = increment("1.0.0-" + "a".repeat(MAX_IDENTIFIER_BYTES), "prerelease");
        if (!valid(sizeOutput)) throw Error("size output");
        if (code(() => increment("1.0.0", "prepatch", "a".repeat(MAX_IDENTIFIER_BYTES + 1))) !== "SEMVER_LIMIT") throw Error("identifier size");
        const total = (n) => "1.0.0-" + ["a".repeat(n), ...Array(30).fill("a".repeat(31))].join(".");
        const near = total(56);
        if (near.length !== MAX_VERSION_BYTES - 2) throw Error("total fixture");
        const output = increment(near, "prerelease");
        if (output.length !== MAX_VERSION_BYTES || !valid(output) || parse(output).version !== output) throw Error("total output");
        if (code(() => increment(total(57), "prerelease")) !== "SEMVER_LIMIT") throw Error("total overflow");
        const core = increment("9999999999999999999.0.0", "major");
        if (!valid(core) || parse(core).version !== core) throw Error("core output");
        if (code(() => increment("18446744073709551615.0.0", "major")) !== "SEMVER_OVERFLOW") throw Error("core overflow");
    "#,
    );
}

#[test]
fn import_needs_no_permission_and_installs_no_global_callbacks() {
    let js = runtime();
    run(
        &js,
        r#"
        import { valid, compare, parse, format, increment, satisfies, validRange } from "neo:semver";
        if (!valid("1.2.3")) throw Error("valid");
        if (compare("1.0.0", "1.0.1") !== -1) throw Error("compare");
        if (parse("1.2.3").major !== "1") throw Error("parse");
        if (format({ major: "1", minor: "2", patch: "3" }) !== "1.2.3") throw Error("format");
        if (increment("1.2.3", "patch") !== "1.2.4") throw Error("increment");
        if (!satisfies("1.2.3", "^1.0.0") || !validRange("*")) throw Error("range");
        "#,
    );
    assert_eq!(
        js.eval(
            "typeof fs + ',' + typeof env + ',' + typeof __neoSemverValid + ',' + \
             typeof __neoSemverCompare"
        )
        .unwrap(),
        "undefined,undefined,undefined,undefined"
    );
}

#[test]
fn an_explicit_allowlist_can_admit_semver_alone() {
    let options = RuntimeOptions::new().modules([RuntimeModule::Semver]);
    let js = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    run(
        &js,
        "import { parse } from 'neo:semver';
         if (parse('1.2.3').patch !== '3') throw Error('semver');",
    );
    let options = RuntimeOptions::with_allowlist(ModuleAllowlist::none());
    let denied = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    assert!(
        denied
            .run_module("denied.js", "import 'neo:semver';")
            .is_err()
    );
}

#[test]
fn tampered_globals_and_prototypes_cannot_reach_private_callbacks() {
    let js = runtime();
    js.run_module(
        "tamper.js",
        r#"
        Object.defineProperty(Object.prototype, "major", {
          configurable: true,
          set() { throw Error("poisoned major setter"); },
        });
        Object.defineProperty(Object.prototype, "length", {
          configurable: true,
          get() { throw Error("poisoned length getter"); },
        });
        Object.defineProperty(Array.prototype, "0", {
          configurable: true,
          set() { throw Error("poisoned index setter"); },
        });
        Object.defineProperty(Array.prototype, "push", {
          configurable: true,
          value() { throw Error("poisoned push"); },
        });
        Array.isArray = () => false;
        Number.isSafeInteger = () => false;
        JSON.parse = () => { throw Error("poisoned JSON.parse"); };
        Object.create = () => { throw Error("poisoned Object.create"); };
        Object.defineProperty = () => { throw Error("poisoned Object.defineProperty"); };
        globalThis.__neoSemverValid = () => false;
        globalThis.__neoSemverCompare = () => 99;
        "#,
    )
    .unwrap();
    js.run_module(
        "main.js",
        r#"
        import { valid, compare, parse, format, increment, satisfies } from "neo:semver";
        if (!valid("1.2.3")) throw Error("replaced valid callback");
        if (compare("1.0.0", "1.0.1") !== -1) throw Error("replaced compare callback");
        const value = parse("1.2.3-alpha.1+build");
        if (value.major !== "1" || value.prerelease[0] !== "alpha") throw Error("parse");
        if (format(value) !== value.version) throw Error("format");
        if (increment("1.2.3", "patch") !== "1.2.4") throw Error("increment");
        if (!satisfies("1.2.3", "^1.0.0")) throw Error("range");
        "#,
    )
    .unwrap();
}

#[test]
fn typescript_entry_can_use_the_documented_types() {
    let js = runtime();
    js.run_module(
        "main.ts",
        r#"
        import { parse, format, increment, satisfies } from "neo:semver";
        import type { SemVerVersion } from "neo:semver";
        const value: SemVerVersion = parse("1.2.3-alpha.1+build.5");
        const major: string = value.major;
        if (major !== "1") throw Error("parse");
        if (format(value) !== value.version) throw Error("format");
        if (increment(value.version, "patch") !== "1.2.4") throw Error("increment");
        if (!satisfies(value.version, "^1.2.3-alpha")) throw Error("range");
        "#,
    )
    .unwrap();
}
