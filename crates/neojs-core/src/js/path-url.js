// File-only subset of the WHATWG URL API. Other schemes and URLSearchParams
// are not implemented. Only host canonicalization crosses into native code.
function fileUrlApi(parseHost) {
  const Type = TypeError;
  const Range = RangeError;
  const Uri = URIError;
  const define = Object.defineProperty;
  const symbolTag = Symbol.toStringTag;
  const encode = encodeURIComponent;
  const decode = decodeURIComponent;
  const bind = (fn) => Function.prototype.call.bind(fn);
  const code = bind(String.prototype.charCodeAt);
  const slice = bind(String.prototype.slice);
  const lower = bind(String.prototype.toLowerCase);
  const index = bind(String.prototype.indexOf);
  const wellFormed = bind(String.prototype.toWellFormed);
  const join = bind(Array.prototype.join);
  const exec = bind(RegExp.prototype.exec);
  const limit = 64 * 1024;
  const hostLimit = 8 * 1024;
  const hex = '0123456789ABCDEF';
  const pathEncodeSet = '"<>#?^`{}';
  const queryEncodeSet = '"#<>\'';
  const fragmentEncodeSet = '"<>`';
  const drivePattern = /^[A-Za-z][:|]$/;
  const schemePattern = /^[A-Za-z][A-Za-z0-9+.-]*:/;
  const hexPairPattern = /^[0-9A-Fa-f]{2}$/;
  const drivePathPattern = /^\\*([A-Za-z]:)(?:\\|$)/;

  const invalid = () => new Type('Invalid file URL');
  const bounded = (text, max = limit) => {
    if (text.length > max) throw new Range('File URL size limit exceeded');
    return text;
  };
  const escapeByte = (c) => '%' + hex[c >> 4] + hex[c & 15];
  // charCodeAt returns NaN past the end; a bracket probe can instead
  // invoke an inherited numeric getter on String.prototype.
  const separator = (text, offset) => {
    const c = code(text, offset);
    return c === 47 || c === 92;
  };
  const drive = (s) => typeof s === 'string' && s.length === 2 &&
    exec(drivePattern, s) !== null;
  const startsDrive = (s) => drive(slice(s, 0, 2)) && (s.length === 2 || separator(s, 2));

  // URL parsing trims surrounding C0 controls and spaces; setters do not.
  function clean(text, trim = false) {
    let start = 0;
    let end = text.length;
    if (trim) {
      while (start < end && code(text, start) <= 32) ++start;
      while (end > start && code(text, end - 1) <= 32) --end;
    }
    let result = '';
    for (let i = start; i < end; ++i) {
      const c = code(text, i);
      if (c !== 9 && c !== 10 && c !== 13) result += text[i];
    }
    return result;
  }

  function percentEncode(text, extra) {
    let result = '';
    for (let i = 0; i < text.length; ++i) {
      const c = code(text, i);
      if (c >= 127) {
        const width = c >= 0xD800 && c <= 0xDBFF ? 2 : 1;
        result += encode(slice(text, i, i + width));
        i += width - 1;
      } else {
        result += c <= 32 || index(extra, text[i]) !== -1 ? escapeByte(c) : text[i];
      }
    }
    return result;
  }

  function canonicalHost(raw) {
    if (raw === '') return '';
    const host = parseHost(bounded(raw, hostLimit));
    return host == null ? null : host === 'localhost' ? '' : host;
  }

  function shorten(path) {
    if (path.length > 1 || (path.length === 1 && !drive(path[0]))) --path.length;
  }

  // Preserve each empty segment. In particular, a file URL with an empty host
  // and pathname "//a" serializes as "file:////a", not "file:///a".
  function segments(text, path) {
    let start = 0;
    for (let i = 0; i <= text.length; ++i) {
      if (i !== text.length && !separator(text, i)) continue;
      const segment = slice(text, start, i);
      const folded = lower(segment);
      const parent = folded === '..' || folded === '.%2e' ||
        folded === '%2e.' || folded === '%2e%2e';
      if (parent) shorten(path);
      if (parent || folded === '.' || folded === '%2e') {
        if (i === text.length) append(path, '');
      } else {
        append(path, path.length === 0 && drive(segment)
          ? segment[0] + ':' : percentEncode(segment, pathEncodeSet));
      }
      start = i + 1;
    }
    return path;
  }

  function append(path, segment) {
    // Avoid inherited numeric setters on Array.prototype.
    define(path, path.length, { __proto__: null, value: segment, writable: true,
      enumerable: true, configurable: true });
  }

  function checked(host, path, query, fragment) {
    const pathname = '/' + join(path, '/');
    const href = bounded('file://' + host + pathname +
      (query === null ? '' : '?' + query) + (fragment === null ? '' : '#' + fragment));
    return { host, path, pathname, query, fragment, href };
  }

  function parse(input, base = null) {
    let text = clean(input, true);
    const scheme = exec(schemePattern, text);
    if (scheme !== null) {
      if (lower(scheme[0]) !== 'file:') throw invalid();
      text = slice(text, scheme[0].length);
    } else if (base === null) {
      throw invalid();
    }

    const hash = index(text, '#');
    const fragment = hash < 0 ? null : percentEncode(slice(text, hash + 1), fragmentEncodeSet);
    if (hash >= 0) text = slice(text, 0, hash);
    const question = index(text, '?');
    const query = question < 0 ? null : percentEncode(slice(text, question + 1), queryEncodeSet);
    if (question >= 0) text = slice(text, 0, question);

    let host = '';
    let path = [];
    if (separator(text, 0) && separator(text, 1)) {
      let end = 2;
      while (end < text.length && !separator(text, end)) ++end;
      const authority = slice(text, 2, end);
      if (drive(authority)) {
        text = slice(text, 2);
      } else {
        host = canonicalHost(authority);
        if (host === null) throw invalid();
        text = slice(text, end + 1);
      }
    } else if (separator(text, 0)) {
      text = slice(text, 1);
      if (base !== null) {
        host = base.host;
        if (base.path.length > 0 && !startsDrive(text) && drive(base.path[0])) {
          append(path, base.path[0]);
        }
      }
    } else if (base !== null) {
      host = base.host;
      if (text === '') return checked(host, base.path, query === null ? base.query : query, fragment);
      if (!startsDrive(text)) {
        // Do not use Array.prototype.slice's species lookup on mutable
        // intrinsics. The base path array never leaves this closure.
        for (let i = 0; i < base.path.length; ++i) append(path, base.path[i]);
        shorten(path);
      }
    }
    return checked(host, segments(text, path), query, fragment);
  }

  let recordOf;
  let createUrl;
  class URL {
    #record;

    static {
      recordOf = (value) => value !== null && typeof value === 'object' && #record in value
        ? value.#record : null;
      createUrl = (record) => {
        const url = new URL('file:///');
        url.#record = record;
        return url;
      };
    }

    constructor(input, base = undefined) {
      const baseRecord = base === undefined ? null : parse(urlText(base));
      this.#record = parse(urlText(input), baseRecord);
    }

    get href() { return this.#record.href; }
    set href(value) { this.#record = parse(urlText(value)); }
    get protocol() { this.#record; return 'file:'; }
    set protocol(value) { this.#record; urlText(value); }
    get origin() { this.#record; return 'null'; }
    get username() { this.#record; return ''; }
    set username(value) { this.#record; urlText(value); }
    get password() { this.#record; return ''; }
    set password(value) { this.#record; urlText(value); }
    get port() { this.#record; return ''; }
    set port(value) { this.#record; urlText(value); }
    get host() { return this.#record.host; }
    set host(value) { this.#setHost(value); }
    get hostname() { return this.#record.host; }
    set hostname(value) { this.#setHost(value); }
    get pathname() { return this.#record.pathname; }
    set pathname(value) { this.#setPath(urlText(value)); }
    get search() { return this.#record.query ? '?' + this.#record.query : ''; }
    set search(value) { this.#setSuffix(value, '?'); }
    get hash() { return this.#record.fragment ? '#' + this.#record.fragment : ''; }
    set hash(value) { this.#setSuffix(value, '#'); }
    toString() { return this.#record.href; }
    toJSON() { return this.#record.href; }

    #setHost(value) {
      const { path, query, fragment } = this.#record;
      const text = clean(urlText(value));
      let end = 0;
      while (end < text.length && !separator(text, end) && text[end] !== '?' && text[end] !== '#') ++end;
      const host = canonicalHost(slice(text, 0, end));
      if (host !== null) this.#record = checked(host, path, query, fragment);
    }

    #setPath(text) {
      const { host, query, fragment } = this.#record;
      text = clean(text);
      this.#record = checked(host, segments(separator(text, 0) ? slice(text, 1) : text, []), query, fragment);
    }

    #setSuffix(value, prefix) {
      const { host, path, query, fragment } = this.#record;
      const text = urlText(value);
      const suffix = text === '' ? null : percentEncode(
        clean(text[0] === prefix ? slice(text, 1) : text),
        prefix === '?' ? queryEncodeSet : fragmentEncodeSet);
      this.#record = prefix === '?'
        ? checked(host, path, suffix, fragment) : checked(host, path, query, suffix);
    }
  }

  function urlText(value) {
    if (typeof value === 'string') return wellFormed(bounded(value));
    const record = recordOf(value);
    if (record !== null) return record.href;
    throw new Type('File URL input must be a string or a URL from this realm');
  }

  function toFileUrl(path, windows, isAbsolute) {
    if (typeof path !== 'string') throw new Type('File URL conversion requires a path string');
    if (!isAbsolute(bounded(path))) throw new Type('File URL conversion requires an absolute path');
    path = wellFormed(path);
    let start = 0;
    let host = '';
    if (windows) {
      // Exactly two separators, nonempty server, and a single separator
      // before the share (or an empty share at the end), as upstream.
      if (separator(path, 0) && separator(path, 1) && !separator(path, 2)) {
        let end = 2;
        while (end < path.length && !separator(path, end)) ++end;
        if (end > 2 && end < path.length && !separator(path, end + 1)) {
          const raw = slice(path, 2, end);
          host = canonicalHost(clean(raw));
          if (host === null || (host === '' && raw !== 'localhost')) throw invalid();
          start = end;
        }
      }
    } else {
      while (start + 1 < path.length && code(path, start + 1) === 47) ++start;
    }
    let escaped = '';
    for (let i = start; i < path.length; ++i) {
      const c = code(path, i);
      escaped += c === 37 || (!windows && c === 92) || (c >= 9 && c <= 13) || c === 32
        ? escapeByte(c) : path[i];
    }
    const pathname = segments(separator(escaped, 0) ? slice(escaped, 1) : escaped, []);
    return createUrl(checked(host, pathname, null, null));
  }

  function fromFileUrl(input, windows) {
    const record = typeof input === 'string' ? parse(wellFormed(checkNul(bounded(input)))) : recordOf(input);
    if (record === null) throw new Type('Expected a file URL string or a URL from this realm');
    const { pathname, host } = record;
    let escaped = '';
    for (let i = 0; i < pathname.length; ++i) {
      const c = pathname[i];
      if (c === '%' && exec(hexPairPattern, slice(pathname, i + 1, i + 3)) === null) {
        escaped += '%25';
      } else {
        escaped += windows && c === '/' ? '\\' : c;
      }
    }
    let path;
    try {
      path = decode(escaped);
    } catch {
      throw new Uri('File URL pathname has invalid UTF-8');
    }
    checkNul(path);
    if (!windows) return path;
    // A real WHATWG file URL can have both a host and a drive-shaped first
    // segment. Keep the separator before that segment in a UNC result.
    if (host !== '') return '\\\\' + host + path;
    const root = exec(drivePathPattern, path);
    return root === null ? path : root[1] + '\\' + slice(path, root[0].length);
  }

  function checkNul(text) {
    if (index(text, '\0') !== -1) throw new Type('File URL conversion does not accept NUL');
    return text;
  }

  define(URL.prototype, symbolTag, { __proto__: null, value: 'URL', configurable: true });
  define(globalThis, 'URL', { __proto__: null, value: URL, writable: true, configurable: true });
  return { toFileUrl, fromFileUrl };
}
