// Copyright 2018-2025 the Deno authors. MIT license.
// Adapted from @neotales/path. See path.LICENSE.md for full notices and revision.
// This file and path.js run in the same private factory; initialization is in path.js.

const GLOB_LENGTH_LIMIT = 4096;
const GLOB_PART_LIMIT = 256;
const GLOB_SOURCE_LIMIT = 16384;
const GLOB_DEPTH_LIMIT = 16;
const GLOB_BRANCH_LIMIT = 64;
const GLOB_REPEAT_LIMIT = 6;
const GLOB_SUBJECT_LIMIT = 16384;
const GLOB_MATCH_WORK_LIMIT = 250000;

function globError(ErrorType, code, message) {
  const error = new ErrorType(message);
  defineProperty(error, 'code', { __proto__: null, value: code, enumerable: true });
  return error;
}

/** Write a private slot without running an inherited numeric setter. */
function setGlobSlot(array, index, value) {
  defineProperty(array, index, {
    __proto__: null, value, writable: true, enumerable: true, configurable: true,
  });
}

function globLimit(message) {
  throw globError(NativeRangeError, 'PATH_GLOB_LIMIT', message);
}

function globSyntax(message) {
  throw globError(NativeSyntaxError, 'PATH_GLOB_SYNTAX', message);
}

function globText(value, name, limit = GLOB_LENGTH_LIMIT) {
  // Bound the input before scanning it for NUL.
  if (typeof value === 'string' && value.length > limit)
    globLimit(name + ' exceeds the glob length limit');
  return assertString(value, name);
}

function globOptions(value) {
  if (value === undefined) return undefined;
  if (value === null || typeof value !== 'object' || isArray(value))
    throw new NativeTypeError('glob options must be an object');
  return value;
}

function globBoolean(options, name, fallback) {
  if (options === undefined || !hasOwn(options, name)) return fallback;
  const value = options[name];
  if (value === undefined) return fallback;
  if (typeof value !== 'boolean') throw new NativeTypeError(name + ' must be a boolean');
  return value;
}

/** The shared upstream heuristic, not a syntax check or a dialect-aware parser. */
function detectGlob(value) {
  let text = globText(value, 'glob');
  const detector =
    /\\(.)|(^!|\*|\?|[\].+)]\?|\[[^[\\\]]+\]|\{[^{\\}]+\}|\(\?[:!=][^\\)]+\)|\([^(|]+\|[^\\)]+\)|@\([^)]+\))/;
  let result;
  while ((result = regExpExec(detector, text)) !== null) {
    if (result[2]) return true;
    let next = result.index + result[0].length;
    const open = result[1];
    const close = open === '{' ? '}' : open === '(' ? ')' : open === '[' ? ']' : '';
    if (close !== '') {
      const end = indexOf(text, close, next);
      if (end !== -1) next = end + 1;
    }
    text = slice(text, next);
  }
  return false;
}

/** Reuse the lexical normalizer, protecting only raw `**`/`..` pairs. */
function normalizeGlobText(text, normalize, isSeparator) {
  let result = '';
  let start = 0;
  let afterGlobstar = false;
  for (let i = 0; i <= text.length; ++i) {
    if (i < text.length && !isSeparator(charCodeAt(text, i))) continue;
    const segment = slice(text, start, i);
    if (segment !== '') {
      result += afterGlobstar && segment === '..' ? '\u0000' : segment;
      afterGlobstar = segment === '**';
    }
    if (i < text.length) result += text[i];
    start = i + 1;
  }
  // User NULs have already been rejected; the private marker cannot collide.
  // Restore it without looking up a replace/split hook on user prototypes.
  const normalized = normalize(result);
  let offset = 0;
  result = '';
  for (let marker = indexOf(normalized, '\u0000'); marker !== -1;
    marker = indexOf(normalized, '\u0000', offset)) {
    result += slice(normalized, offset, marker) + '..';
    offset = marker + 1;
  }
  return result + slice(normalized, offset);
}

function escapeGlobLiteral(character) {
  return indexOf('!$()*+.=?[\\]^{|}', character) === -1 ? character : '\\' + character;
}

function globClass(name) {
  switch (name) {
    case 'alnum': return '\\dA-Za-z';
    case 'alpha': return 'A-Za-z';
    case 'ascii': return '\\x00-\\x7F';
    case 'blank': return '\\t ';
    case 'cntrl': return '\\x00-\\x1F\\x7F';
    case 'digit': return '\\d';
    case 'graph': return '\\x21-\\x7E';
    case 'lower': return 'a-z';
    case 'print': return '\\x20-\\x7E';
    case 'punct': return '!\"#$%&\'()*+,\\-./:;<=>?@[\\\\\\]^_‘{|}~';
    case 'space': return '\\s\\v';
    case 'upper': return 'A-Z';
    case 'word': return '\\w';
    case 'xdigit': return '\\dA-Fa-f';
    default: return globSyntax('unknown glob character class');
  }
}

/**
 * Parse one segment using the upstream group's spelling and class table.
 * Store a small program as well as regex source. The boolean matcher can
 * then count its work without relying on the engine's backtracking.
 */
function compileGlob(text, dialect, extended, globstar, flags) {
  const { separator, isSeparator, escapePrefix } = dialect;
  const noSep = dialect.noSep;
  const program = [];
  let branchCost = 1;
  let repeats = 0;
  let cursor = 0;

  function boundedRepeat() {
    if (++repeats > GLOB_REPEAT_LIMIT) globLimit('too many variable glob repetitions');
  }

  function atomRegex(source) {
    try {
      return new NativeRegExp('^' + source + '$', flags);
    } catch (error) {
      if (!(error instanceof NativeSyntaxError)) throw error;
      throw globError(NativeSyntaxError, 'PATH_GLOB_SYNTAX', 'invalid glob character range');
    }
  }

  function readClass() {
    let source = '[';
    if (charAt(text, cursor) === '!') {
      source += '^';
      ++cursor;
    }
    let entries = 0;
    while (cursor < text.length && !isSeparator(charCodeAt(text, cursor))) {
      const character = text[cursor++];
      if (character === ']') {
        if (entries === 0) globSyntax('empty glob character class');
        return source + ']';
      }
      ++entries;
      if (character === escapePrefix) {
        if (cursor === text.length || isSeparator(charCodeAt(text, cursor)))
          globSyntax('dangling glob escape');
        const literal = text[cursor++];
        source += indexOf('-\\]^', literal) === -1 ? literal : '\\' + literal;
      } else if (character === '[' && charAt(text, cursor) === ':') {
        ++cursor;
        const start = cursor;
        while (cursor < text.length && text[cursor] !== ':' &&
          !isSeparator(charCodeAt(text, cursor))) ++cursor;
        if (charAt(text, cursor) !== ':' || charAt(text, cursor + 1) !== ']')
          globSyntax('unclosed named glob character class');
        source += globClass(slice(text, start, cursor));
        cursor += 2;
      } else {
        source += character === '^' || character === '[' ? '\\' + character : character;
      }
    }
    return globSyntax('unclosed glob character class');
  }

  function sourceOf(sequence) {
    let source = '';
    for (let i = 0; i < sequence.length; ++i) {
      source += sequence[i].source;
      if (source.length > GLOB_SOURCE_LIMIT) globLimit('generated glob regex is too long');
    }
    return source;
  }

  // Repeating variable-width or overlapping alternatives can give an
  // exponential number of decompositions, even with a short regex source.
  function checkRepeatedGroup(arms) {
    const prefixes = [];
    for (let i = 0; i < arms.length; ++i) {
      const arm = arms[i];
      if (arm.length === 0) globLimit('repeating glob alternatives must not be empty');
      let prefix = '';
      for (let j = 0; j < arm.length; ++j) {
        if (arm[j].kind !== 'atom')
          globLimit('repeating glob alternatives must have fixed width');
        if (prefix.length === j && arm[j].literal !== undefined) prefix += arm[j].literal;
      }
      if (arms.length === 1) continue;
      if (prefix === '') globLimit('repeating glob alternatives must have distinct literal prefixes');
      const insensitive = [];
      for (let j = 0; j < prefixes.length; ++j) {
        const other = prefixes[j];
        const width = prefix.length < other.length ? prefix.length : other.length;
        let k = 0;
        while (k < width) {
          if (prefix[k] !== other[k]) {
            if (flags !== 'i') break;
            if (!hasOwn(insensitive, k)) setGlobSlot(insensitive, k, atomRegex(arm[k].source));
            if (regExpExec(insensitive[k], other[k]) === null) break;
          }
          ++k;
        }
        if (k === width) globLimit('repeating glob alternatives must have distinct literal prefixes');
      }
      setGlobSlot(prefixes, prefixes.length, prefix);
    }
  }

  function readGroup(operator, depth) {
    if (depth > GLOB_DEPTH_LIMIT) globLimit('glob groups are nested too deeply');
    const brace = operator === '{';
    const end = brace ? '}' : ')';
    const divider = brace ? ',' : '|';
    const arms = [];
    let source = '';
    while (true) {
      const arm = readSequence(end, divider, depth);
      setGlobSlot(arms, arms.length, arm);
      if (arms.length > GLOB_BRANCH_LIMIT) globLimit('too many glob alternatives');
      source += (arms.length === 1 ? '' : '|') + sourceOf(arm);
      if (source.length > GLOB_SOURCE_LIMIT) globLimit('generated glob regex is too long');
      if (charAt(text, cursor) !== divider) break;
      ++cursor;
    }
    if (charAt(text, cursor++) !== end) globSyntax('unclosed glob group or separator inside a glob group');
    branchCost *= arms.length + (operator === '?' ? 1 : 0);
    if (branchCost > GLOB_BRANCH_LIMIT) globLimit('too many glob alternative combinations');
    if (operator === '*' || operator === '+' || operator === '!') boundedRepeat();
    if (operator === '*' || operator === '+') checkRepeatedGroup(arms);
    if (operator === '!') source = '(?!' + source + ')' + noSep + '*';
    else source = '(?:' + source + ')' + (operator === '@' || brace ? '' : operator);
    return { __proto__: null, kind: 'group', arms, operator, source };
  }

  function readSequence(end, divider, depth) {
    const sequence = [];
    while (cursor < text.length && !isSeparator(charCodeAt(text, cursor))) {
      if (text[cursor] === end || text[cursor] === divider) break;
      const character = text[cursor++];
      let token;
      if (character === '{') {
        token = readGroup('{', depth + 1);
      } else if (extended && charAt(text, cursor) === '(' && indexOf('*+?@!', character) !== -1) {
        ++cursor;
        token = readGroup(character, depth + 1);
      } else if (character === '*') {
        while (charAt(text, cursor) === '*') ++cursor;
        boundedRepeat();
        token = { __proto__: null, kind: 'star', min: 0, source: noSep + '*' };
      } else if (character === '[') {
        const range = readClass();
        token = { __proto__: null, kind: 'atom', regex: atomRegex(range),
          source: '(?!' + separator + ')' + range };
      } else if (character === '?') {
        token = { __proto__: null, kind: 'atom', regex: /^(?:.)$/, source: '(?!' + separator + ').' };
      } else {
        let literal = character;
        if (character === escapePrefix) {
          if (cursor === text.length || isSeparator(charCodeAt(text, cursor)))
            globSyntax('dangling glob escape');
          literal = text[cursor++];
        }
        const source = escapeGlobLiteral(literal);
        token = { __proto__: null, kind: 'atom', literal, source };
      }
      setGlobSlot(sequence, sequence.length, token);
    }
    return sequence;
  }

  // Like the reference, ignore trailing separators on non-root patterns.
  let end = text.length;
  while (end > 1 && isSeparator(charCodeAt(text, end - 1))) --end;
  text = slice(text, 0, end);
  const trailing = { __proto__: null, kind: 'separator', min: 0, source: separator + '*' };
  const required = { __proto__: null, kind: 'separator', min: 1, source: separator + '+' };
  const rest = { __proto__: null, kind: 'rest', source: '[\\s\\S]*' };
  if (isSeparator(charCodeAt(text, 0))) {
    setGlobSlot(program, program.length, required);
    while (isSeparator(charCodeAt(text, cursor))) ++cursor;
  }
  while (cursor < text.length) {
    const after = cursor + 2;
    if (globstar && text[cursor] === '*' && charAt(text, cursor + 1) === '*' &&
      (after === text.length || isSeparator(charCodeAt(text, after)))) {
      cursor = after;
      while (isSeparator(charCodeAt(text, cursor))) ++cursor;
      if (program.length > 0 && program[program.length - 1].kind === 'globstar') {
        // Adjacent globstars have the same language as a single globstar.
        if (cursor === text.length)
          setGlobSlot(program, program.length - 1, rest);
      } else {
        boundedRepeat();
        setGlobSlot(program, program.length, cursor === text.length
          ? rest
          : { __proto__: null, kind: 'globstar', source: '(?:' + noSep + '*' + separator + ')*' });
      }
    } else {
      const segment = readSequence(undefined, undefined, 0);
      for (let i = 0; i < segment.length; ++i) setGlobSlot(program, program.length, segment[i]);
      setGlobSlot(program, program.length, cursor < text.length ? required : trailing);
      while (isSeparator(charCodeAt(text, cursor))) ++cursor;
    }
  }
  const source = '^' + sourceOf(program) + '$';
  if (source.length > GLOB_SOURCE_LIMIT) globLimit('generated glob regex is too long');
  return { program, source };
}

/** Find reachable offsets, counting even duplicate transitions against the budget. */
function testGlob(program, path, isSeparator) {
  let work = GLOB_MATCH_WORK_LIMIT;
  function step() {
    if (--work < 0) globLimit('glob match exceeded its work limit');
  }
  function makeOffsets() {
    return { offsets: [], seen: createNullObject(null) };
  }
  function add(result, offset) {
    step();
    if (result.seen[offset] === undefined) {
      result.seen[offset] = true;
      setGlobSlot(result.offsets, result.offsets.length, offset);
    }
  }
  function evaluate(sequence, offsets) {
    for (let i = 0; i < sequence.length && offsets.length !== 0; ++i) {
      const result = makeOffsets();
      for (let j = 0; j < offsets.length; ++j) advance(sequence[i], offsets[j], result);
      offsets = result.offsets;
    }
    return offsets;
  }
  function alternatives(node, start, result) {
    for (let i = 0; i < node.arms.length; ++i) {
      step();
      const ends = evaluate(node.arms[i], [start]);
      if (node.operator === '!' && ends.length !== 0) return true;
      for (let j = 0; j < ends.length; ++j) add(result, ends[j]);
    }
    return false;
  }
  function advance(node, start, result) {
    step();
    const kind = node.kind;
    if (kind === 'atom') {
      if (start < path.length && !isSeparator(charCodeAt(path, start)) &&
        (node.literal === undefined ? regExpExec(node.regex, path[start]) !== null :
          node.literal === path[start])) add(result, start + 1);
    } else if (kind === 'rest') {
      add(result, path.length);
    } else if (kind === 'globstar') {
      add(result, start);
      for (let i = start; i < path.length; ++i) {
        step();
        if (isSeparator(charCodeAt(path, i))) add(result, i + 1);
      }
    } else if (kind === 'separator' || kind === 'star') {
      if (!node.min) add(result, start);
      for (let i = start; i < path.length &&
        isSeparator(charCodeAt(path, i)) === (kind === 'separator'); ++i) add(result, i + 1);
    } else {
      const operator = node.operator;
      if (operator === '?' || operator === '*') add(result, start);
      if (operator === '*' || operator === '+') {
        const reached = makeOffsets();
        alternatives(node, start, reached);
        for (let i = 0; i < reached.offsets.length; ++i) {
          const offset = reached.offsets[i];
          add(result, offset);
          alternatives(node, offset, reached);
        }
      } else if (operator !== '!') {
        alternatives(node, start, result);
      } else if (!alternatives(node, start, undefined)) {
        add(result, start);
        for (let i = start; i < path.length && !isSeparator(charCodeAt(path, i)); ++i)
          add(result, i + 1);
      }
    }
  }
  const ends = evaluate(program, [0]);
  for (let i = 0; i < ends.length; ++i) {
    if (ends[i] === path.length) return true;
  }
  return false;
}

/** Build lexical glob helpers for one separator grammar. */
function globApi(windows, normalize) {
  const isSeparator = windows ? isWin32Separator : isPosixSeparator;
  const dialect = {
    isSeparator,
    separator: windows ? '[\\\\/]' : '/',
    noSep: windows ? '[^\\\\/]' : '[^/]',
    escapePrefix: windows ? '`' : '\\',
  };

  /** Compile a bounded glob to a fresh anchored RegExp without doing path I/O. */
  function globToRegExp(glob, options) {
    const text = globText(glob, 'glob');
    const settings = globOptions(options);
    const extended = globBoolean(settings, 'extended', true);
    const globstar = globBoolean(settings, 'globstar', true);
    const flags = globBoolean(settings, 'caseInsensitive', false) ? 'i' : '';
    if (text === '') return /(?!)/;
    return new NativeRegExp(compileGlob(text, dialect, extended, globstar, flags).source, flags);
  }

  /** Normalize lexically, optionally protecting `..` immediately after `**`. */
  function normalizeGlob(glob, options) {
    const text = globText(glob, 'glob');
    return globBoolean(globOptions(options), 'globstar', false)
      ? normalizeGlobText(text, normalize, isSeparator)
      : normalize(text);
  }

  /** Join a bounded array of glob strings using the namespace separator. */
  function joinGlobs(globs, options) {
    const length = arrayLength(globs, 'globs');
    if (length > GLOB_PART_LIMIT) globLimit('too many glob parts');
    const globstar = globBoolean(globOptions(options), 'globstar', false);
    const parts = [];
    let size = 0;
    for (let i = 0; i < length; ++i) {
      if (!hasOwn(globs, i)) throw new NativeTypeError('glob parts must be present');
      const part = globText(globs[i], 'glob part');
      if (part === '') continue;
      size += part.length + (parts.length === 0 ? 0 : 1);
      if (size > GLOB_LENGTH_LIMIT) globLimit('joined glob is too long');
      setGlobSlot(parts, parts.length, part);
    }
    if (parts.length === 0) return '.';
    if (!globstar && windows) return joinWin32Parts(parts);
    const text = joinArray(parts, windows ? '\\' : '/');
    return globstar ? normalizeGlobText(text, normalize, isSeparator) : normalize(text);
  }

  /** Match with compile defaults and a deterministic work budget, without cwd. */
  function match(path, pattern) {
    const subject = globText(path, 'path', GLOB_SUBJECT_LIMIT);
    const text = globText(pattern, 'glob');
    return text !== '' && testGlob(compileGlob(text, dialect, true, true, '').program, subject, isSeparator);
  }

  return { isGlob: detectGlob, globToRegExp, normalizeGlob, joinGlobs, match, matchesGlob: match };
}
