//! End-to-end tests for `neo:dotenv` and the `neo:env.has` addition.
//!
//! The parser is pure. These tests run it with every permission denied, then
//! assert privacy, object safety, ordering, and error behaviour through the real
//! module loader.

use neojs_core::{NeoJs, Permissions};

#[test]
fn inherited_descriptor_hooks_cannot_observe_parsed_values() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-descriptor-hooks.js",
        r#"
        const token = 'synthetic-token-7c1f';
        const define = Object.defineProperty;
        const create = Object.create;
        const jsonParse = JSON.parse;
        let observed = false;
        function install() {
            for (const field of ['get', 'set']) {
                define(Object.prototype, field, {
                    __proto__: null, configurable: true,
                    get() {
                        if (this.value === token) observed = true;
                        return undefined;
                    },
                    set(value) { if (value === token) observed = true; },
                });
            }
        }
        function remove() {
            delete Object.prototype.get;
            delete Object.prototype.set;
        }
        function verify(module) {
            const result = module.parse('TOKEN=' + token + '\n__proto__=safe');
            if (result.TOKEN !== token || result.__proto__ !== 'safe' ||
                Object.getPrototypeOf(result) !== null) throw Error('result');
            const descriptor = Object.getOwnPropertyDescriptor(result, 'TOKEN');
            if (descriptor.value !== token || !descriptor.enumerable ||
                !descriptor.writable || !descriptor.configurable) throw Error('descriptor');
            try {
                module.parse('A=' + token + '\nBROKEN\nB=\0');
                throw Error('expected parse error');
            } catch (error) {
                if (!(error instanceof module.DotenvParseError) ||
                    error.code !== 'DOTENV_SYNTAX' || error.line !== 2 || error.column !== 7)
                    throw error;
                for (const field of ['name', 'code', 'line', 'column']) {
                    const d = Object.getOwnPropertyDescriptor(error, field);
                    if (!d || !d.writable || !d.configurable ||
                        d.enumerable !== (field !== 'name')) throw Error('error descriptor');
                }
            }
            if (observed) throw Error('parsed value reached inherited descriptor hook');
        }
        install();
        Object.defineProperty = Object.create = JSON.parse = () => { throw Error('replaced intrinsic'); };
        try {
            const module = await import('neo:dotenv');
            verify(module);
            remove();
            install();
            verify(module);
        } finally {
            remove();
            Object.defineProperty = define;
            Object.create = create;
            JSON.parse = jsonParse;
        }
        "#,
    )
    .unwrap();
}

#[test]
fn parse_needs_no_permissions_and_protects_object_keys() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-keys.js",
        r#"
        import { parse } from 'neo:dotenv';
        const parsed = parse('__proto__=safe\nconstructor=ctor\nprototype=proto\nA=1\nA=2\nB=日本');
        if (Object.getPrototypeOf(parsed) !== null) throw Error('prototype is not null');
        if (parsed['__proto__'] !== 'safe') throw Error('__proto__ value');
        if (parsed.constructor !== 'ctor') throw Error('constructor value');
        if (parsed.prototype !== 'proto') throw Error('prototype value');
        if (parsed.A !== '2') throw Error('duplicate did not win');
        if (Object.keys(parsed).join(',') !== '__proto__,constructor,prototype,A,B')
            throw Error('key order: ' + Object.keys(parsed).join(','));
        if (parsed.B !== '日本') throw Error('unicode');
        if (({}).polluted !== undefined) throw Error('prototype polluted');
        if (typeof env !== 'undefined' || typeof fs !== 'undefined')
            throw Error('unrequested capability');
        "#,
    )
    .unwrap();
}

#[test]
fn enumeration_is_javascript_order_for_numeric_and_duplicate_keys() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-order.js",
        r#"
        import { parse } from 'neo:dotenv';
        const parsed = parse('10=ten\n2=two\nb=bee\na=aye\n02=leading\na=last\n2=twice\n10=tenth');
        if (Object.keys(parsed).join(',') !== '2,10,b,a,02')
            throw Error('enumeration: ' + Object.keys(parsed).join(','));
        if (parsed['2'] !== 'twice' || parsed['10'] !== 'tenth') throw Error('numeric values');
        if (parsed.a !== 'last' || parsed['02'] !== 'leading') throw Error('string values');
        "#,
    )
    .unwrap();
}

#[test]
fn parse_keeps_values_literal_and_never_expands() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-literal.js",
        r#"
        import { parse } from 'neo:dotenv';
        const parsed = parse('A=$HOME\nB=${HOME:-/tmp}\nC=$(echo hi)\nD=`echo hi`\nE="\\n\\u2603"');
        if (parsed.A !== '$HOME') throw Error('A: ' + parsed.A);
        if (parsed.B !== '${HOME:-/tmp}') throw Error('B: ' + parsed.B);
        if (parsed.C !== '$(echo hi)') throw Error('C: ' + parsed.C);
        if (parsed.D !== 'echo hi') throw Error('D: ' + parsed.D);
        if (parsed.E !== '\n☃') throw Error('E: ' + JSON.stringify(parsed.E));
        "#,
    )
    .unwrap();
}

#[test]
fn host_callback_is_private_before_and_after_import() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-private.js",
        r#"
        if ('__neoDotenvParse' in globalThis) throw Error('host callback is global');
        if (typeof globalThis.__neoDotenvParse !== 'undefined') throw Error('host callback value');

        // Replace any tempting global before the module is evaluated. The
        // module must use its private callback, not this function.
        globalThis.__neoDotenvParse = function () {
            return JSON.stringify({ok: true, entries: [['FORGED', 'yes']]});
        };
        const module = await import('neo:dotenv');
        const values = module.parse('A=1');
        if (values.A !== '1' || values.FORGED !== undefined) throw Error('forged result');
        if (module.default !== module.parse) throw Error('default export');
        for (const name of Object.keys(module)) {
            if (name !== 'default' && name !== 'parse' && name !== 'DotenvParseError' &&
                name !== 'MAX_SOURCE_BYTES' && name !== 'MAX_ENTRIES' && name !== 'MAX_KEY_BYTES') {
                throw Error('unexpected export: ' + name);
            }
        }

        // A later replacement must not affect the captured private callback.
        globalThis.__neoDotenvParse = function () {
            return JSON.stringify({ok: true, entries: [['LATER', 'yes']]});
        };
        const later = module.parse('B=2');
        if (later.B !== '2' || later.LATER !== undefined) throw Error('later forgery');
        delete globalThis.__neoDotenvParse;
        "#,
    )
    .unwrap();
}

#[test]
fn parse_rejects_malformed_input_with_stable_locations() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-errors.js",
        r#"
        import { DotenvParseError, parse, MAX_SOURCE_BYTES } from 'neo:dotenv';

        function expect(source, code, line, column) {
            try {
                parse(source);
            } catch (error) {
                if (!(error instanceof DotenvParseError)) throw Error('not DotenvParseError');
                if (error.name !== 'DotenvParseError') throw Error('name');
                if (error.code !== code) throw Error('code: ' + error.code);
                if (error.line !== line || error.column !== column)
                    throw Error('location: ' + error.line + ':' + error.column);
                return;
            }
            throw Error('expected a throw');
        }

        expect('A="secret', 'DOTENV_QUOTE', 1, 3);
        expect('A="\\u12"', 'DOTENV_ESCAPE', 1, 4);
        expect('A=1\nBROKEN\n', 'DOTENV_SYNTAX', 2, 7);
        expect('A=1\nB=2\n=bad', 'DOTENV_KEY', 3, 1);
        expect('A=\0B', 'DOTENV_NUL', 1, 3);
        expect('A="\\u0000"', 'DOTENV_NUL', 1, 4);
        expect('\ud800', 'DOTENV_ENCODING', 1, 1);

        // The first source-order error wins; a later NUL or surrogate must not
        // hide an earlier syntax error.
        expect('BROKEN\nA=\0B', 'DOTENV_SYNTAX', 1, 7);
        expect('A=\0B\nBROKEN', 'DOTENV_NUL', 1, 3);
        expect('BROKEN\n\ud800', 'DOTENV_SYNTAX', 1, 7);
        expect('\ud800\nBROKEN', 'DOTENV_ENCODING', 1, 1);
        expect('BROKEN\nA="\\u0000"', 'DOTENV_SYNTAX', 1, 7);
        expect('A="\\u0000"\nBROKEN', 'DOTENV_NUL', 1, 4);

        // CRLF, lone CR, and astral characters keep normalized scalar columns.
        expect('A=1\r\nBROKEN', 'DOTENV_SYNTAX', 2, 7);
        expect('A=1\rBROKEN', 'DOTENV_SYNTAX', 2, 7);
        expect('A=1\r\n\ud800', 'DOTENV_ENCODING', 2, 1);
        expect('A=1\r\nB=\0', 'DOTENV_NUL', 2, 3);
        expect('A=😀\nBROKEN😀', 'DOTENV_SYNTAX', 2, 7);
        expect('A=😀\n\ud800', 'DOTENV_ENCODING', 2, 1);
        expect('A=😀\ud800', 'DOTENV_ENCODING', 1, 4);

        try {
            parse('A="secret');
            throw Error('expected a throw');
        } catch (error) {
            if (String(error.message).includes('secret')) throw Error('message leaked a value');
        }

        let typeError = false;
        try { parse(1); } catch (error) { typeError = error instanceof TypeError; }
        if (!typeError) throw Error('expected TypeError');
        typeError = false;
        try { parse(new String('A=1')); } catch (error) { typeError = error instanceof TypeError; }
        if (!typeError) throw Error('expected boxed String TypeError');

        // The length read must be the string's own length, not a prototype
        // property. A wrong numeric length here would reject a valid source.
        Object.prototype.length = 2000000;
        try {
            if (parse('A=12345').A !== '12345') throw Error('length hook');
        } finally {
            delete Object.prototype.length;
        }

        if (MAX_SOURCE_BYTES !== 1048576) throw Error('limit constant');
        "#,
    )
    .unwrap();
}

#[test]
fn native_bounds_both_utf16_length_and_exact_utf8_bytes() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "dotenv-limits.js",
        r#"
        import { parse, MAX_SOURCE_BYTES } from 'neo:dotenv';
        function expectLimit(source) {
            try {
                parse(source);
            } catch (error) {
                if (error.code !== 'DOTENV_LIMIT') throw Error('code: ' + error.code);
                return;
            }
            throw Error('expected DOTENV_LIMIT');
        }
        expectLimit('A=' + 'x'.repeat(MAX_SOURCE_BYTES + 1));
        // 600,000 UTF-16 units is under the unit bound but over 1 MiB as UTF-8.
        expectLimit('A=' + 'é'.repeat(600000));
        // Original UTF-8 bytes exceed 1 MiB even though normalization would
        // reduce the text below that bound.
        expectLimit('A=' + 'é'.repeat(400000) + '\r\n'.repeat(200000));
        const near = parse('A=' + 'x'.repeat(900000));
        if (near.A.length !== 900000) throw Error('near-limit value');
        "#,
    )
    .unwrap();
}

#[test]
fn env_module_is_absent_without_permission_and_has_is_available_with_it() {
    let denied = NeoJs::new(Permissions::default()).unwrap();
    assert!(
        denied
            .run_module("denied-env.js", "import 'neo:env';")
            .is_err()
    );

    let allowed = NeoJs::new(Permissions {
        environment: true,
        ..Permissions::default()
    })
    .unwrap();
    allowed
        .run_module(
            "allowed-env.js",
            "import { get, has } from 'neo:env';
             if (typeof get !== 'function' || typeof has !== 'function') throw Error('missing');
             if (has('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== false) throw Error('has absent');
             if (get('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== undefined) throw Error('get absent');
             if (env.get('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== undefined) throw Error('global get');
             if (typeof env.has !== 'function') throw Error('global has');",
        )
        .unwrap();
}
