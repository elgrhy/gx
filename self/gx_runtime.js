// GX Runtime — prepended to every compiled GX program
"use strict";
function gx_str(v) {
  if (v === null || v === undefined) return "null";
  if (v === true)  return "true";
  if (v === false) return "false";
  if (Array.isArray(v)) return "[" + v.map(gx_str).join(", ") + "]";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}
function gx_add(a, b) {
  if (typeof a === "number" && typeof b === "number") return a + b;
  return String(a) + String(b);
}
function gx_len(v) {
  if (Array.isArray(v) || typeof v === "string") return v.length;
  if (v && typeof v === "object") return Object.keys(v).length;
  return 0;
}
function gx_range(start, end, step) {
  if (arguments.length === 1) { end = start; start = 0; }
  step = step || 1;
  const arr = [];
  for (let i = start; step > 0 ? i < end : i > end; i += step) arr.push(i);
  return arr;
}
function gx_push(arr, v) { return [...arr, v]; }
function gx_set_key(obj, key, val) { return Object.assign({}, obj, {[key]: val}); }
function gx_is_digit(c)      { return /^[0-9]$/.test(c); }
function gx_is_alpha(c)      { return /^[a-zA-Z]$/.test(c); }
function gx_is_alnum(c)      { return /^[a-zA-Z0-9]$/.test(c); }
function gx_is_whitespace(c) { return /^\s$/.test(c); }
function gx_assert(cond, msg) { if (!cond) throw new Error(msg || "Assertion failed"); }
function __builtin_env(name) { return process.env[name] ?? null; }
function read_file(path) { try { return require('fs').readFileSync(path, 'utf8'); } catch(e) { return null; } }
function file_exists(path) { return require('fs').existsSync(path); }
function write_file(path, content) { require('fs').writeFileSync(path, content, 'utf8'); }
