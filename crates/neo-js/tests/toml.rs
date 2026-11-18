//! End-to-end tests for `neo:toml`.
//!
//! The parser is pure. These tests run it with every permission denied, then
//! assert privacy, object safety, limits, and error behaviour through the real
//! module loader.

use neo_js::{NeoJs, Permissions};

#[test]
fn parses_nested_types_without_permissions() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-types.js",
        r#"
        import { parse } from 'neo:toml';
        const value = parse(
          'title = "héllo ☃"\n' +
          'count = 42\n' +
          'ratio = 1.5\n' +
          'enabled = true\n' +
          'nothing = []\n' +
          'server = { host = "localhost", ports = [80, 443] }\n' +
          '[[items]]\nname = "one"\n' +
          '[[items]]\nname = "two"\n'
        );
        if (Object.getPrototypeOf(value) !== null) throw Error('root prototype');
        if (Object.getPrototypeOf(value.server) !== null) throw Error('nested prototype');
        if (!Array.isArray(value.nothing)) throw Error('array');
        if (value.title !== 'héllo ☃') throw Error('unicode');
        if (value.count !== 42 || value.ratio !== 1.5 || value.enabled !== true)
          throw Error('scalars');
        if (value.server.ports[1] !== 443) throw Error('nested array');
        if (value.items[1].name !== 'two') throw Error('array of tables');
        if (typeof fs !== 'undefined' || typeof env !== 'undefined')
          throw Error('unrequested capability');
        "#,
    )
    .unwrap();
}

#[test]
fn hostile_keys_are_ordinary_own_properties() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-keys.js",
        r#"
        import { parse } from 'neo:toml';
        const value = parse('__proto__ = "safe"\nconstructor = "ctor"\nprototype = "proto"\n');
        if (Object.getPrototypeOf(value) !== null) throw Error('prototype');
        if (value['__proto__'] !== 'safe') throw Error('__proto__');
        if (value.constructor !== 'ctor') throw Error('constructor');
        if (value.prototype !== 'proto') throw Error('prototype key');
        if (Object.keys(value).join(',') !== '__proto__,constructor,prototype')
          throw Error('keys: ' + Object.keys(value).join(','));
        if (({}).polluted !== undefined) throw Error('prototype polluted');
        "#,
    )
    .unwrap();
}

#[test]
fn rejects_duplicates_integers_datetimes_and_non_finite_floats() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-errors.js",
        r#"
        import { TomlParseError, parse } from 'neo:toml';
        function expect(source, code, line, column) {
          try {
            parse(source);
          } catch (error) {
            if (!(error instanceof TomlParseError)) throw Error('not TomlParseError');
            if (error.name !== 'TomlParseError') throw Error('name');
            if (error.code !== code) throw Error('code: ' + error.code);
            if (error.line !== line || error.column !== column)
              throw Error('location: ' + error.line + ':' + error.column);
            return;
          }
          throw Error('expected a throw');
        }
        expect('a = 1\na = 2\n', 'TOML_SYNTAX', 2, 1);
        expect('[a]\n[a]\n', 'TOML_SYNTAX', 2, 1);
        expect('a = 9007199254740992', 'TOML_INTEGER', 0, 0);
        expect('a = -9007199254740992', 'TOML_INTEGER', 0, 0);
        expect('a = inf', 'TOML_FLOAT', 0, 0);
        expect('a = -inf', 'TOML_FLOAT', 0, 0);
        expect('a = nan', 'TOML_FLOAT', 0, 0);
        expect('a = 1979-05-27T07:32:00Z', 'TOML_DATETIME', 0, 0);
        expect('a = 1979-05-27', 'TOML_DATETIME', 0, 0);
        expect('a = 07:32:00', 'TOML_DATETIME', 0, 0);
        try {
          parse('secret = "synthetic-secret-7c1f"\nbroken = [\n');
          throw Error('expected a throw');
        } catch (error) {
          if (String(error.message).includes('synthetic-secret-7c1f'))
            throw Error('message leaked a value');
        }
        "#,
    )
    .unwrap();
}

#[test]
fn encoding_errors_win_over_later_budget_errors() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-order.js",
        r#"
        import { parse } from 'neo:toml';
        function expect(source, code, line, column) {
          try {
            parse(source);
          } catch (error) {
            if (error.code !== code) throw Error('code: ' + error.code);
            if (error.line !== line || error.column !== column)
              throw Error('location: ' + error.line + ':' + error.column);
            return;
          }
          throw Error('expected ' + code);
        }

        expect('a = "x\u0000y"', 'TOML_NUL', 1, 7);

        const wide = 'a = "\ud800"\nb = [' + '0,'.repeat(100000) + '0]';
        expect(wide, 'TOML_ENCODING', 1, 6);
        "#,
    )
    .unwrap();
}

#[test]
fn enforces_source_depth_and_collection_limits() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-limits.js",
        r#"
        import { MAX_DEPTH, MAX_SOURCE_BYTES, parse } from 'neo:toml';
        function expectLimit(source, code, line, column) {
          try {
            parse(source);
          } catch (error) {
            if (error.code !== code) throw Error('code: ' + error.code);
            if (line !== undefined && (error.line !== line || error.column !== column))
              throw Error('location: ' + error.line + ':' + error.column);
            return error;
          }
          throw Error('expected ' + code);
        }
        const nested = 'a = ' + '['.repeat(MAX_DEPTH + 1) + '1' + ']'.repeat(MAX_DEPTH + 1);
        expectLimit(nested, 'TOML_DEPTH');
        // The source-size limit has no event and reports the documented 1:1.
        expectLimit('A'.repeat(MAX_SOURCE_BYTES + 1), 'TOML_LIMIT', 1, 1);
        const lateDepth = expectLimit('x = 1\ny = 2\n' + nested, 'TOML_DEPTH');
        if (lateDepth.line !== 3 || lateDepth.column <= 1)
          throw Error('late depth location: ' + lateDepth.line + ':' + lateDepth.column);
        const wide = 'a = [' + '0,'.repeat(100000) + '0]';
        const wideError = expectLimit(wide, 'TOML_LIMIT');
        if (wideError.line !== 1 || wideError.column <= 1)
          throw Error('wide location: ' + wideError.line + ':' + wideError.column);
        const lateWide = expectLimit('x = 1\ny = 2\n' + wide, 'TOML_LIMIT');
        if (lateWide.line !== 3 || lateWide.column <= 1)
          throw Error('late wide location: ' + lateWide.line + ':' + lateWide.column);
        let rows = '';
        for (let index = 0; index < 100001; index++) rows += 'a' + index + ' = 1\n';
        expectLimit(rows, 'TOML_LIMIT');
        if (MAX_SOURCE_BYTES !== 1048576 || MAX_DEPTH !== 64) throw Error('constants');
        "#,
    )
    .unwrap();
}

#[test]
fn intrinsic_replacement_cannot_forge_or_observe_values() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-private.js",
        r#"
        if ('__neoTomlParse' in globalThis) throw Error('host callback is global');
        globalThis.__neoTomlParse = () => JSON.stringify({ok: true, value: {FORGED: 'yes'}});

        const token = 'synthetic-token-7c1f';
        const define = Object.defineProperty;
        const create = Object.create;
        const jsonParse = JSON.parse;
        const objectKeys = Object.keys;
        const isArray = Array.isArray;
        let observed = false;
        function install() {
          for (const field of ['get', 'set']) {
            define(Object.prototype, field, {
              __proto__: null, configurable: true,
              get() { if (this.value === token) observed = true; return undefined; },
              set(value) { if (value === token) observed = true; },
            });
          }
        }
        function remove() {
          delete Object.prototype.get;
          delete Object.prototype.set;
        }
        install();
        Object.defineProperty = Object.create = JSON.parse = Object.keys = Array.isArray =
          () => { throw Error('replaced intrinsic'); };
        try {
          const module = await import('neo:toml');
          if (module.default !== module.parse) throw Error('default export');
          const value = module.parse('TOKEN = "' + token + '"\n__proto__ = "safe"');
          if (value.TOKEN !== token || value['__proto__'] !== 'safe' ||
              Object.getPrototypeOf(value) !== null) throw Error('result');
          const descriptor = Object.getOwnPropertyDescriptor(value, 'TOKEN');
          if (descriptor.value !== token || !descriptor.enumerable ||
              !descriptor.writable || !descriptor.configurable) throw Error('descriptor');
          if (module.parse('A = 1').FORGED !== undefined) throw Error('forged result');
        } finally {
          remove();
          Object.defineProperty = define;
          Object.create = create;
          JSON.parse = jsonParse;
          Object.keys = objectKeys;
          Array.isArray = isArray;
        }
        if (observed) throw Error('parsed value reached inherited descriptor hook');
        "#,
    )
    .unwrap();
}

#[test]
fn non_strings_throw_type_errors() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "toml-types-args.js",
        r#"
        import { parse } from 'neo:toml';
        for (const value of [1, null, undefined, {}, [], new String('a = 1')]) {
          let thrown = false;
          try { parse(value); } catch (error) { thrown = error instanceof TypeError; }
          if (!thrown) throw Error('expected TypeError');
        }
        "#,
    )
    .unwrap();
}
