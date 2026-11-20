//! Native JavaScript disposal in entry modules, with no host resources.

use neojs_core::{ModuleAllowlist, NeoJs, Permissions, RuntimeOptions};

#[test]
fn using_disposes_in_reverse_order_when_an_entry_finishes_or_throws() {
    for (entry, setup) in [
        ("using.js", "const firstLabel = 'first';"),
        ("using.ts", "const firstLabel: string = 'first';"),
    ] {
        for fail in [false, true] {
            let js = NeoJs::new_with_options(
                Permissions::default(),
                RuntimeOptions::with_allowlist(ModuleAllowlist::none()),
            )
            .unwrap();
            let exit = if fail {
                "throw Error('body failure');"
            } else {
                ""
            };
            let source = format!(
                "{setup}
                 globalThis.events = [];
                 function resource(label) {{
                     const value = {{
                         [Symbol.dispose]() {{
                             if (this !== value) throw Error('wrong disposer receiver');
                             globalThis.events.push(label);
                         }}
                     }};
                     return value;
                 }}
                 using first = resource(firstLabel);
                 using second = resource('second');
                 globalThis.events.push('body');
                 {exit}"
            );
            let result = js.run_module(entry, &source);
            if fail {
                let error = result.unwrap_err();
                assert!(error.contains("body failure"), "{entry}: {error}");
            } else {
                result.unwrap();
            }
            assert_eq!(
                js.eval("globalThis.events.join(',')").unwrap(),
                "body,second,first",
                "{entry}, fail={fail}"
            );
        }
    }
}
