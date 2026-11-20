//! End-to-end tests for `neo:yaml`.
//!
//! The parser is pure. These tests run it with every permission denied, then
//! assert privacy, object safety, alias and duplicate rejection, limits, and
//! error behaviour through the real module loader.

use neojs_core::{NeoJs, Permissions};

#[test]
fn parses_nested_types_without_permissions() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-types.js",
        r#"
        import { parse } from 'neo:yaml';
        const value = parse(
          'title: héllo ☃\n' +
          'count: 42\n' +
          'ratio: 1.5\n' +
          'enabled: true\n' +
          'nothing: []\n' +
          'server:\n  host: localhost\n  ports: [80, 443]\n' +
          'items:\n  - name: one\n  - name: two\n'
        );
        if (Object.getPrototypeOf(value) !== null) throw Error('root prototype');
        if (Object.getPrototypeOf(value.server) !== null) throw Error('nested prototype');
        if (!Array.isArray(value.nothing)) throw Error('array');
        if (value.title !== 'héllo ☃') throw Error('unicode');
        if (value.count !== 42 || value.ratio !== 1.5 || value.enabled !== true)
          throw Error('scalars');
        if (value.server.ports[1] !== 443) throw Error('nested array');
        if (value.items[1].name !== 'two') throw Error('sequence of mappings');
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
        "yaml-keys.js",
        r#"
        import { parse } from 'neo:yaml';
        const value = parse('__proto__: safe\nconstructor: ctor\nprototype: proto\n');
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
fn rejects_aliases_tags_duplicate_keys_and_multiple_documents() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-errors.js",
        r#"
        import { YamlParseError, parse } from 'neo:yaml';
        function expect(source, code, line, column) {
          try {
            parse(source);
          } catch (error) {
            if (!(error instanceof YamlParseError)) throw Error('not YamlParseError');
            if (error.name !== 'YamlParseError') throw Error('name');
            if (error.code !== code) throw Error('code: ' + error.code);
            if (error.line !== line || error.column !== column)
              throw Error('location: ' + error.line + ':' + error.column);
            return;
          }
          throw Error('expected a throw');
        }
        expect('a: 1\na: 2\n', 'YAML_DUPLICATE', 2, 1);
        expect('a: 1\n"a": 2\n', 'YAML_DUPLICATE', 2, 1);
        expect('a: &a [1, 2]\nb: *a\n', 'YAML_ALIAS', 2, 4);
        expect('a: &a [*a]\n', 'YAML_ALIAS', 1, 8);
        expect('a: !!str 1\n', 'YAML_TAG', 1, 10);
        expect('a: !custom 1\n', 'YAML_TAG', 1, 12);
        expect('1: value\n', 'YAML_KEY', 1, 1);
        expect('true: value\n', 'YAML_KEY', 1, 1);
        expect('? [a, b]\n: value\n', 'YAML_KEY', 1, 3);
        expect('---\na: 1\n---\nb: 2\n', 'YAML_MULTIDOC', 3, 1);
        expect('a: .inf\n', 'YAML_FLOAT', 1, 4);
        expect('a: 9007199254740992\n', 'YAML_INTEGER', 1, 4);
        expect('a: [1,\n', 'YAML_SYNTAX', 2, 1);
        try {
          parse('secret: synthetic-secret-7c1f\nbroken: [\n');
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
fn encoding_errors_win_over_later_budget_errors_and_positions_survive_crlf() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-order.js",
        r#"
        import { parse } from 'neo:yaml';
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

        // A NUL is rejected at its own position, not at the later parse point.
        expect('a: 1\u0000\n', 'YAML_NUL', 1, 5);

        // A lone surrogate is decoded to U+FFFD and recorded. The parser sees
        // valid text after it and would otherwise reach the node budget much
        // later; the earlier encoding error must win.
        let wide = 'a: "\ud800"\n';
        for (let index = 0; index < 60000; index++) wide += 'k' + index + ': 1\n';
        expect(wide, 'YAML_ENCODING', 1, 5);

        // CRLF is normalized and an astral scalar counts as one column.
        expect('a: 😀\r\nb: [1,\r\n', 'YAML_SYNTAX', 3, 1);
        "#,
    )
    .unwrap();
}

#[test]
fn alias_bomb_is_rejected_before_expansion() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-alias-bomb.js",
        r#"
        import { parse } from 'neo:yaml';
        let source = 'a: &a [x, x, x, x, x, x, x, x, x, x]\n';
        for (let index = 0; index < 30; index++) {
          const previous = String.fromCharCode(97 + index);
          const next = String.fromCharCode(98 + index);
          source += next + ': &' + next + ' [';
          for (let repeat = 0; repeat < 100; repeat++) source += '*' + previous + ',';
          source += ']\n';
        }
        let rejected = false;
        try {
          parse(source);
        } catch (error) {
          if (error.code !== 'YAML_ALIAS') throw Error('code: ' + error.code);
          rejected = true;
        }
        if (!rejected) throw Error('expected YAML_ALIAS');
        "#,
    )
    .unwrap();
}

#[test]
fn enforces_source_depth_and_collection_limits() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-limits.js",
        r#"
        import { MAX_DEPTH, MAX_SOURCE_BYTES, parse } from 'neo:yaml';
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
        const nested = '['.repeat(MAX_DEPTH + 1) + '1' + ']'.repeat(MAX_DEPTH + 1);
        expectLimit(nested, 'YAML_DEPTH');
        // The source-size limit has no event and reports the documented 1:1.
        expectLimit('A'.repeat(MAX_SOURCE_BYTES + 1), 'YAML_LIMIT', 1, 1);
        expectLimit('- x\n'.repeat(100001), 'YAML_LIMIT');
        const lateLimit = expectLimit('a:\n' + '  - x\n'.repeat(100001), 'YAML_LIMIT');
        if (lateLimit.line <= 1 || lateLimit.column <= 1)
          throw Error('late limit location: ' + lateLimit.line + ':' + lateLimit.column);
        const lateDepth = expectLimit(
          'a: 1\nb: ' + '['.repeat(MAX_DEPTH + 1) + '1' + ']'.repeat(MAX_DEPTH + 1) + '\n',
          'YAML_DEPTH'
        );
        if (lateDepth.line !== 2 || lateDepth.column <= 1)
          throw Error('late depth location: ' + lateDepth.line + ':' + lateDepth.column);
        if (MAX_SOURCE_BYTES !== 1048576 || MAX_DEPTH !== 64) throw Error('constants');
        "#,
    )
    .unwrap();
}

#[test]
fn date_like_scalars_remain_strings() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-dates.js",
        r#"
        import { parse } from 'neo:yaml';
        const value = parse('date: 2020-01-01\ntime: 2020-01-01T00:00:00Z\n');
        if (value.date !== '2020-01-01') throw Error('date');
        if (value.time !== '2020-01-01T00:00:00Z') throw Error('time');
        "#,
    )
    .unwrap();
}

#[test]
fn intrinsic_replacement_cannot_forge_or_observe_values() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "yaml-private.js",
        r#"
        if ('__neoYamlParse' in globalThis) throw Error('host callback is global');
        globalThis.__neoYamlParse = () => JSON.stringify({ok: true, value: {FORGED: 'yes'}});

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
          const module = await import('neo:yaml');
          if (module.default !== module.parse) throw Error('default export');
          const value = module.parse('TOKEN: ' + token + '\n__proto__: safe\n');
          if (value.TOKEN !== token || value['__proto__'] !== 'safe' ||
              Object.getPrototypeOf(value) !== null) throw Error('result');
          const descriptor = Object.getOwnPropertyDescriptor(value, 'TOKEN');
          if (descriptor.value !== token || !descriptor.enumerable ||
              !descriptor.writable || !descriptor.configurable) throw Error('descriptor');
          if (module.parse('A: 1').FORGED !== undefined) throw Error('forged result');
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
        "yaml-types-args.js",
        r#"
        import { parse } from 'neo:yaml';
        for (const value of [1, null, undefined, {}, [], new String('a: 1')]) {
          let thrown = false;
          try { parse(value); } catch (error) { thrown = error instanceof TypeError; }
          if (!thrown) throw Error('expected TypeError');
        }
        "#,
    )
    .unwrap();
}
