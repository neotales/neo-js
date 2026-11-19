# Third-party notices for `neo:path`

`crates/neo-js/src/js/path.js` adapts algorithms from `@neotales/path` in the
`js-libraries` repository.

- Upstream: https://github.com/neotales/js-libraries
- Upstream tree:
  https://github.com/neotales/js-libraries/tree/c7e88644f45a78f65bf639c9d365bb9eff21bcfe/std/jsr/path
- Directory: `std/jsr/path`
- Revision: `c7e88644f45a78f65bf639c9d365bb9eff21bcfe` (2026-09-28)
- Last change touching this directory:
  `2656f91e43b183927d1fdc3c501e77ab4b1ccce4` (2026-09-27)
- License: MIT
- License file: `std/jsr/path/LICENSE.md` at that revision, SHA-256
  `4a9edd22e4358ff7607cfd6e056e8640712b5e60100a5c339f842e02d3ad8acb`
- Checkout state: `git status` and `git diff` were clean for `std/jsr/path` when
  inspected, and the license file matches the recorded revision.

Adapted source files, algorithms only; neo-js changed the namespace shape,
argument validation, NUL handling, and several documented edge cases:

- `posix/_util.ts`, `posix/normalize.ts`, `posix/join.ts`, `posix/basename.ts`,
  `posix/dirname.ts`, `posix/extname.ts`, `posix/is_absolute.ts`,
  `posix/parse.ts`, `posix/format.ts`, `posix/resolve.ts`, `posix/constants.ts`
- `windows/_util.ts`, `windows/normalize.ts`, `windows/join.ts`,
  `windows/basename.ts`, `windows/dirname.ts`, `windows/extname.ts`,
  `windows/is_absolute.ts`, `windows/parse.ts`, `windows/format.ts`,
  `windows/resolve.ts`, `windows/to_namespaced_path.ts`, `windows/constants.ts`
- `_common/normalize_string.ts`, `_common/basename.ts`, `_common/dirname.ts`,
  `_common/format.ts`, `_common/strip_trailing_separators.ts`,
  `_common/assert_path.ts`, `_common/common.ts`
- `types.ts`

The adapted files also carry header lines, recorded here without alteration:
`// Copyright 2018-2025 the Deno authors. MIT license.`,
`// Copyright 2018-2026 the Deno authors. MIT license.` (one file), and
`// Copyright the Browserify authors. MIT License.` The complete license text
from the revision follows verbatim.

---

# MIT License

Copyright 2022-2026 Neotales

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the “Software”), to deal in
the Software without restriction, including without limitation the
rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

---

MIT License

Copyright 2018-2022 the Deno authors.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

---

MIT License

Copyright (c) 2013 James Halliday

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
