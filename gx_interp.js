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

function is_ident_start(c) {
  return (gx_is_alpha(c) || (c == "_"));
}
function is_ident_cont(c) {
  return (gx_is_alnum(c) || (c == "_"));
}
function make_token(kind, value, line) {
  return {"kind": kind, "value": value, "line": line};
}
function tokenize(src) {
  var LBRACE = String.fromCharCode(123);
  var RBRACE = String.fromCharCode(125);
  var tokens = [];
  var i = 0;
  var line = 1;
  var n = gx_len(src);
  while ((i < n))   {
    var c = src[i];
    if ((c == "\n"))     {
      var line = gx_add(line, 1);
      var i = gx_add(i, 1);
      continue;
    }
    if (gx_is_whitespace(c))     {
      var i = gx_add(i, 1);
      continue;
    }
    if ((((c == "/") && (gx_add(i, 1) < n)) && (src[gx_add(i, 1)] == "/")))     {
      while (((i < n) && (src[i] != "\n")))       {
        var i = gx_add(i, 1);
      }
      continue;
    }
    if ((c == "\""))     {
      var i = gx_add(i, 1);
      var start = i;
      var buf = "";
      while (((i < n) && (src[i] != "\"")))       {
        if (((src[i] == "\\") && (gx_add(i, 1) < n)))         {
          var next = src[gx_add(i, 1)];
          if ((next == "n"))           {
            var buf = gx_add(buf, "\n");
          } else if ((next == "t"))           {
            var buf = gx_add(buf, "\t");
          } else if ((next == "r"))           {
            var buf = gx_add(buf, "\\r");
          } else if ((next == "\\"))           {
            var buf = gx_add(buf, "\\");
          } else if ((next == "\""))           {
            var buf = gx_add(buf, "\"");
          } else           {
            var buf = gx_add(gx_add(buf, "\\"), next);
          }
          var i = gx_add(i, 2);
        } else         {
          var buf = gx_add(buf, src[i]);
          var i = gx_add(i, 1);
        }
      }
      var i = gx_add(i, 1);
      var tokens = gx_push(tokens, make_token("Str", buf, line));
      continue;
    }
    if ((gx_is_digit(c) || (((c == "-") && (gx_add(i, 1) < n)) && gx_is_digit(src[gx_add(i, 1)]))))     {
      var start = i;
      var buf = "";
      if ((c == "-"))       {
        var buf = gx_add(buf, c);
        var i = gx_add(i, 1);
      }
      while (((i < n) && gx_is_digit(src[i])))       {
        var buf = gx_add(buf, src[i]);
        var i = gx_add(i, 1);
      }
      if (((((i < n) && (src[i] == ".")) && (gx_add(i, 1) < n)) && gx_is_digit(src[gx_add(i, 1)])))       {
        var buf = gx_add(buf, ".");
        var i = gx_add(i, 1);
        while (((i < n) && gx_is_digit(src[i])))         {
          var buf = gx_add(buf, src[i]);
          var i = gx_add(i, 1);
        }
      }
      var tokens = gx_push(tokens, make_token("Num", buf, line));
      continue;
    }
    if (is_ident_start(c))     {
      var buf = "";
      while (((i < n) && is_ident_cont(src[i])))       {
        var buf = gx_add(buf, src[i]);
        var i = gx_add(i, 1);
      }
      var kind = ident_kind(buf);
      var tokens = gx_push(tokens, make_token(kind, buf, line));
      continue;
    }
    if ((gx_add(i, 1) < n))     {
      var two = gx_add(src[i], src[gx_add(i, 1)]);
      if ((two == "=="))       {
        var tokens = gx_push(tokens, make_token("EqEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "!="))       {
        var tokens = gx_push(tokens, make_token("NotEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "<="))       {
        var tokens = gx_push(tokens, make_token("LtEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == ">="))       {
        var tokens = gx_push(tokens, make_token("GtEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "+="))       {
        var tokens = gx_push(tokens, make_token("PlusEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "-="))       {
        var tokens = gx_push(tokens, make_token("MinusEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "*="))       {
        var tokens = gx_push(tokens, make_token("MulEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "/="))       {
        var tokens = gx_push(tokens, make_token("DivEq", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "|>"))       {
        var tokens = gx_push(tokens, make_token("Pipe", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "??"))       {
        var tokens = gx_push(tokens, make_token("NullCoal", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "&&"))       {
        var tokens = gx_push(tokens, make_token("And", two, line));
        var i = gx_add(i, 2);
        continue;
      }
      if ((two == "||"))       {
        var tokens = gx_push(tokens, make_token("Or", two, line));
        var i = gx_add(i, 2);
        continue;
      }
    }
    if ((c == "="))     {
      var tokens = gx_push(tokens, make_token("Eq", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "+"))     {
      var tokens = gx_push(tokens, make_token("Plus", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "-"))     {
      var tokens = gx_push(tokens, make_token("Minus", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "*"))     {
      var tokens = gx_push(tokens, make_token("Star", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "/"))     {
      var tokens = gx_push(tokens, make_token("Slash", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "%"))     {
      var tokens = gx_push(tokens, make_token("Percent", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "<"))     {
      var tokens = gx_push(tokens, make_token("Lt", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == ">"))     {
      var tokens = gx_push(tokens, make_token("Gt", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "!"))     {
      var tokens = gx_push(tokens, make_token("Bang", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "("))     {
      var tokens = gx_push(tokens, make_token("LParen", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == ")"))     {
      var tokens = gx_push(tokens, make_token("RParen", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == LBRACE))     {
      var tokens = gx_push(tokens, make_token("LBrace", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == RBRACE))     {
      var tokens = gx_push(tokens, make_token("RBrace", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "["))     {
      var tokens = gx_push(tokens, make_token("LBrack", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "]"))     {
      var tokens = gx_push(tokens, make_token("RBrack", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == ","))     {
      var tokens = gx_push(tokens, make_token("Comma", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == "."))     {
      var tokens = gx_push(tokens, make_token("Dot", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == ":"))     {
      var tokens = gx_push(tokens, make_token("Colon", c, line));
      var i = gx_add(i, 1);
      continue;
    }
    if ((c == ";"))     {
      var i = gx_add(i, 1);
      continue;
    }
    var i = gx_add(i, 1);
  }
  var tokens = gx_push(tokens, make_token("EOF", "", line));
  return tokens;
}
function ident_kind(word) {
  if ((word == "if"))   {
    return "If";
  }
  if ((word == "else"))   {
    return "Else";
  }
  if ((word == "while"))   {
    return "While";
  }
  if ((word == "for"))   {
    return "For";
  }
  if ((word == "each"))   {
    return "Each";
  }
  if ((word == "in"))   {
    return "In";
  }
  if ((word == "return"))   {
    return "Return";
  }
  if ((word == "function"))   {
    return "Function";
  }
  if ((word == "true"))   {
    return "True";
  }
  if ((word == "false"))   {
    return "False";
  }
  if ((word == "null"))   {
    return "Null";
  }
  if ((word == "and"))   {
    return "And";
  }
  if ((word == "or"))   {
    return "Or";
  }
  if ((word == "not"))   {
    return "Not";
  }
  if ((word == "log"))   {
    return "Log";
  }
  if ((word == "say"))   {
    return "Say";
  }
  if ((word == "assert"))   {
    return "Assert";
  }
  if ((word == "break"))   {
    return "Break";
  }
  if ((word == "continue"))   {
    return "Continue";
  }
  if ((word == "try"))   {
    return "Try";
  }
  if ((word == "catch"))   {
    return "Catch";
  }
  if ((word == "import"))   {
    return "Import";
  }
  if ((word == "agent"))   {
    return "Agent";
  }
  if ((word == "helper"))   {
    return "Helper";
  }
  if ((word == "when"))   {
    return "When";
  }
  if ((word == "started"))   {
    return "Started";
  }
  if ((word == "changes"))   {
    return "Changes";
  }
  if ((word == "brain"))   {
    return "Brain";
  }
  if ((word == "plan"))   {
    return "Plan";
  }
  if ((word == "execute"))   {
    return "Execute";
  }
  if ((word == "remember"))   {
    return "Remember";
  }
  if ((word == "communicate"))   {
    return "Communicate";
  }
  if ((word == "memory"))   {
    return "Memory";
  }
  if ((word == "re"))   {
    return "Re";
  }
  if ((word == "run"))   {
    return "Run";
  }
  if ((word == "escalate"))   {
    return "Escalate";
  }
  if ((word == "to"))   {
    return "To";
  }
  if ((word == "human"))   {
    return "Human";
  }
  if ((word == "spawn"))   {
    return "Spawn";
  }
  if ((word == "with"))   {
    return "With";
  }
  if ((word == "message"))   {
    return "Message";
  }
  return "Ident";
}
function tok(tokens, pos) {
  return tokens[pos];
}
function tok_kind(tokens, pos) {
  var t = tokens[pos];
  if ((t == null))   {
    return "EOF";
  }
  return t.kind;
}
function tok_val(tokens, pos) {
  var t = tokens[pos];
  if ((t == null))   {
    return "";
  }
  return t.value;
}
function at_end(tokens, pos) {
  return (tok_kind(tokens, pos) == "EOF");
}
function check(tokens, pos, kind) {
  return (tok_kind(tokens, pos) == kind);
}
function eat(tokens, pos, kind) {
  if ((tok_kind(tokens, pos) == kind))   {
    return [gx_add(pos, 1), tokens[pos]];
  }
  var t = tokens[pos];
  var tval = "";
  var tline = 0;
  if ((t != null))   {
    var tval = t.value;
    var tline = t.line;
  }
  console.log(gx_str(gx_add(gx_add(gx_add(gx_add(gx_add(gx_add(gx_add("Parse error: expected ", kind), " got "), tok_kind(tokens, pos)), " ('"), tval), "') at line "), gx_str(tline))));
  return [pos, tokens[pos]];
}
function parse(tokens) {
  var pos = 0;
  var stmts = [];
  while ((!at_end(tokens, pos)))   {
    var r = parse_stmt(tokens, pos);
    var pos = r[0];
    var stmts = gx_push(stmts, r[1]);
  }
  return {"tag": "Program", "stmts": stmts};
}
function parse_stmt(tokens, pos) {
  var k = tok_kind(tokens, pos);
  if ((k == "Function"))   {
    return parse_funcdef(tokens, pos);
  }
  if ((k == "If"))   {
    return parse_if(tokens, pos);
  }
  if ((k == "While"))   {
    return parse_while(tokens, pos);
  }
  if ((k == "For"))   {
    return parse_for(tokens, pos);
  }
  if ((k == "Return"))   {
    return parse_return(tokens, pos);
  }
  if ((k == "Log"))   {
    return parse_log(tokens, pos);
  }
  if ((k == "Say"))   {
    return parse_say(tokens, pos);
  }
  if ((k == "Assert"))   {
    return parse_assert(tokens, pos);
  }
  if ((k == "Break"))   {
    return [gx_add(pos, 1), {"tag": "Break"}];
  }
  if ((k == "Continue"))   {
    return [gx_add(pos, 1), {"tag": "Continue"}];
  }
  if ((k == "Try"))   {
    return parse_try(tokens, pos);
  }
  return parse_assign_or_expr(tokens, pos);
}
function parse_funcdef(tokens, pos) {
  var r0 = eat(tokens, pos, "Function");
  var pos = r0[0];
  var r1 = eat(tokens, pos, "Ident");
  var pos = r1[0];
  var name = r1[1].value;
  var r2 = eat(tokens, pos, "LParen");
  var pos = r2[0];
  var params = [];
  while (((!check(tokens, pos, "RParen")) && (!at_end(tokens, pos))))   {
    var r3 = eat(tokens, pos, "Ident");
    var pos = r3[0];
    var params = gx_push(params, r3[1].value);
    if (check(tokens, pos, "Comma"))     {
      var pos = gx_add(pos, 1);
    }
  }
  var r4 = eat(tokens, pos, "RParen");
  var pos = r4[0];
  var rb = parse_block(tokens, pos);
  var pos = rb[0];
  return [pos, {"tag": "FuncDef", "name": name, "params": params, "body": rb[1]}];
}
function parse_if(tokens, pos) {
  var r0 = eat(tokens, pos, "If");
  var pos = r0[0];
  var rc = parse_expr(tokens, pos);
  var pos = rc[0];
  var rb = parse_block(tokens, pos);
  var pos = rb[0];
  var branches = [{"cond": rc[1], "body": rb[1]}];
  var else_body = null;
  while (check(tokens, pos, "Else"))   {
    var pos = gx_add(pos, 1);
    if (check(tokens, pos, "If"))     {
      var pos = gx_add(pos, 1);
      var rc2 = parse_expr(tokens, pos);
      var pos = rc2[0];
      var rb2 = parse_block(tokens, pos);
      var pos = rb2[0];
      var branches = gx_push(branches, {"cond": rc2[1], "body": rb2[1]});
    } else     {
      var reb = parse_block(tokens, pos);
      var pos = reb[0];
      var else_body = reb[1];
      break;
    }
  }
  return [pos, {"tag": "If", "branches": branches, "else_body": else_body}];
}
function parse_while(tokens, pos) {
  var r0 = eat(tokens, pos, "While");
  var pos = r0[0];
  var rc = parse_expr(tokens, pos);
  var pos = rc[0];
  var rb = parse_block(tokens, pos);
  var pos = rb[0];
  return [pos, {"tag": "While", "cond": rc[1], "body": rb[1]}];
}
function parse_for(tokens, pos) {
  var r0 = eat(tokens, pos, "For");
  var pos = r0[0];
  if (check(tokens, pos, "Each"))   {
    var pos = gx_add(pos, 1);
  }
  var rv = eat(tokens, pos, "Ident");
  var pos = rv[0];
  var ri = eat(tokens, pos, "In");
  var pos = ri[0];
  var re = parse_expr(tokens, pos);
  var pos = re[0];
  var rb = parse_block(tokens, pos);
  var pos = rb[0];
  return [pos, {"tag": "For", "var": rv[1].value, "iter": re[1], "body": rb[1]}];
}
function parse_return(tokens, pos) {
  var r0 = eat(tokens, pos, "Return");
  var pos = r0[0];
  if ((check(tokens, pos, "RBrace") || at_end(tokens, pos)))   {
    return [pos, {"tag": "Return", "value": null}];
  }
  var rv = parse_expr(tokens, pos);
  var pos = rv[0];
  return [pos, {"tag": "Return", "value": rv[1]}];
}
function parse_log(tokens, pos) {
  var r0 = eat(tokens, pos, "Log");
  var pos = r0[0];
  var r1 = eat(tokens, pos, "LParen");
  var pos = r1[0];
  var rv = parse_expr(tokens, pos);
  var pos = rv[0];
  var r2 = eat(tokens, pos, "RParen");
  var pos = r2[0];
  return [pos, {"tag": "Log", "value": rv[1]}];
}
function parse_say(tokens, pos) {
  var r0 = eat(tokens, pos, "Say");
  var pos = r0[0];
  var rv = parse_expr(tokens, pos);
  var pos = rv[0];
  return [pos, {"tag": "Say", "value": rv[1]}];
}
function parse_assert(tokens, pos) {
  var r0 = eat(tokens, pos, "Assert");
  var pos = r0[0];
  var rc = parse_expr(tokens, pos);
  var pos = rc[0];
  var msg = null;
  if (check(tokens, pos, "Str"))   {
    var msg = {"tag": "Str", "value": tokens[pos].value};
    var pos = gx_add(pos, 1);
  }
  return [pos, {"tag": "Assert", "cond": rc[1], "msg": msg}];
}
function parse_try(tokens, pos) {
  var r0 = eat(tokens, pos, "Try");
  var pos = r0[0];
  var rb = parse_block(tokens, pos);
  var pos = rb[0];
  var r1 = eat(tokens, pos, "Catch");
  var pos = r1[0];
  var rv = eat(tokens, pos, "Ident");
  var pos = rv[0];
  var rcb = parse_block(tokens, pos);
  var pos = rcb[0];
  return [pos, {"tag": "TryCatch", "try_body": rb[1], "catch_var": rv[1].value, "catch_body": rcb[1]}];
}
function parse_block(tokens, pos) {
  var r0 = eat(tokens, pos, "LBrace");
  var pos = r0[0];
  var stmts = [];
  while (((!check(tokens, pos, "RBrace")) && (!at_end(tokens, pos))))   {
    var rs = parse_stmt(tokens, pos);
    var pos = rs[0];
    var stmts = gx_push(stmts, rs[1]);
  }
  var r1 = eat(tokens, pos, "RBrace");
  var pos = r1[0];
  return [pos, stmts];
}
function parse_assign_or_expr(tokens, pos) {
  var re = parse_expr(tokens, pos);
  var pos = re[0];
  var expr = re[1];
  var k = tok_kind(tokens, pos);
  if ((k == "Eq"))   {
    var pos = gx_add(pos, 1);
    var rv = parse_expr(tokens, pos);
    var pos = rv[0];
    return [pos, {"tag": "Assign", "target": expr, "value": rv[1]}];
  }
  if ((k == "PlusEq"))   {
    var pos = gx_add(pos, 1);
    var rv = parse_expr(tokens, pos);
    var pos = rv[0];
    return [pos, {"tag": "PlusEq", "target": expr, "value": rv[1]}];
  }
  if ((k == "MinusEq"))   {
    var pos = gx_add(pos, 1);
    var rv = parse_expr(tokens, pos);
    var pos = rv[0];
    return [pos, {"tag": "MinusEq", "target": expr, "value": rv[1]}];
  }
  if ((k == "MulEq"))   {
    var pos = gx_add(pos, 1);
    var rv = parse_expr(tokens, pos);
    var pos = rv[0];
    return [pos, {"tag": "MulEq", "target": expr, "value": rv[1]}];
  }
  if ((k == "DivEq"))   {
    var pos = gx_add(pos, 1);
    var rv = parse_expr(tokens, pos);
    var pos = rv[0];
    return [pos, {"tag": "DivEq", "target": expr, "value": rv[1]}];
  }
  return [pos, {"tag": "ExprStmt", "expr": expr}];
}
function parse_expr(tokens, pos) {
  return parse_null_coal(tokens, pos);
}
function parse_null_coal(tokens, pos) {
  var r = parse_or(tokens, pos);
  var pos = r[0];
  var left = r[1];
  while (check(tokens, pos, "NullCoal"))   {
    var pos = gx_add(pos, 1);
    var r2 = parse_or(tokens, pos);
    var pos = r2[0];
    var left = {"tag": "NullCoal", "left": left, "right": r2[1]};
  }
  return [pos, left];
}
function parse_or(tokens, pos) {
  var r = parse_and(tokens, pos);
  var pos = r[0];
  var left = r[1];
  while (check(tokens, pos, "Or"))   {
    var pos = gx_add(pos, 1);
    var r2 = parse_and(tokens, pos);
    var pos = r2[0];
    var left = {"tag": "BinOp", "op": "or", "left": left, "right": r2[1]};
  }
  return [pos, left];
}
function parse_and(tokens, pos) {
  var r = parse_eq(tokens, pos);
  var pos = r[0];
  var left = r[1];
  while (check(tokens, pos, "And"))   {
    var pos = gx_add(pos, 1);
    var r2 = parse_eq(tokens, pos);
    var pos = r2[0];
    var left = {"tag": "BinOp", "op": "and", "left": left, "right": r2[1]};
  }
  return [pos, left];
}
function parse_eq(tokens, pos) {
  var r = parse_cmp(tokens, pos);
  var pos = r[0];
  var left = r[1];
  var k = tok_kind(tokens, pos);
  if (((k == "EqEq") || (k == "NotEq")))   {
    var op = tokens[pos].value;
    var pos = gx_add(pos, 1);
    var r2 = parse_cmp(tokens, pos);
    var pos = r2[0];
    return [pos, {"tag": "BinOp", "op": op, "left": left, "right": r2[1]}];
  }
  return [pos, left];
}
function parse_cmp(tokens, pos) {
  var r = parse_add(tokens, pos);
  var pos = r[0];
  var left = r[1];
  var k = tok_kind(tokens, pos);
  if (((((k == "Lt") || (k == "Gt")) || (k == "LtEq")) || (k == "GtEq")))   {
    var op = tokens[pos].value;
    var pos = gx_add(pos, 1);
    var r2 = parse_add(tokens, pos);
    var pos = r2[0];
    return [pos, {"tag": "BinOp", "op": op, "left": left, "right": r2[1]}];
  }
  return [pos, left];
}
function parse_add(tokens, pos) {
  var r = parse_mul(tokens, pos);
  var pos = r[0];
  var left = r[1];
  var k = tok_kind(tokens, pos);
  while (((k == "Plus") || (k == "Minus")))   {
    var op = tokens[pos].value;
    var pos = gx_add(pos, 1);
    var r2 = parse_mul(tokens, pos);
    var pos = r2[0];
    var left = {"tag": "BinOp", "op": op, "left": left, "right": r2[1]};
    var k = tok_kind(tokens, pos);
  }
  return [pos, left];
}
function parse_mul(tokens, pos) {
  var r = parse_unary(tokens, pos);
  var pos = r[0];
  var left = r[1];
  var k = tok_kind(tokens, pos);
  while ((((k == "Star") || (k == "Slash")) || (k == "Percent")))   {
    var op = tokens[pos].value;
    var pos = gx_add(pos, 1);
    var r2 = parse_unary(tokens, pos);
    var pos = r2[0];
    var left = {"tag": "BinOp", "op": op, "left": left, "right": r2[1]};
    var k = tok_kind(tokens, pos);
  }
  return [pos, left];
}
function parse_unary(tokens, pos) {
  if (check(tokens, pos, "Not"))   {
    var pos = gx_add(pos, 1);
    var r = parse_unary(tokens, pos);
    var pos = r[0];
    return [pos, {"tag": "Unary", "op": "not", "expr": r[1]}];
  }
  if (check(tokens, pos, "Minus"))   {
    var pos = gx_add(pos, 1);
    var r = parse_unary(tokens, pos);
    var pos = r[0];
    return [pos, {"tag": "Unary", "op": "-", "expr": r[1]}];
  }
  return parse_postfix(tokens, pos);
}
function parse_postfix(tokens, pos) {
  var r = parse_primary(tokens, pos);
  var pos = r[0];
  var base = r[1];
  var go = true;
  while (go)   {
    var k = tok_kind(tokens, pos);
    if ((k == "Dot"))     {
      var pos = gx_add(pos, 1);
      var field_tok = tokens[pos];
      var pos = gx_add(pos, 1);
      if (check(tokens, pos, "LParen"))       {
        var pos = gx_add(pos, 1);
        var ra = parse_arglist(tokens, pos);
        var pos = ra[0];
        var r2 = eat(tokens, pos, "RParen");
        var pos = r2[0];
        var base = {"tag": "MethodCall", "obj": base, "method": field_tok.value, "args": ra[1]};
      } else       {
        var base = {"tag": "Field", "obj": base, "field": field_tok.value};
      }
    } else if ((k == "LBrack"))     {
      var pos = gx_add(pos, 1);
      var ri = parse_expr(tokens, pos);
      var pos = ri[0];
      var r2 = eat(tokens, pos, "RBrack");
      var pos = r2[0];
      var base = {"tag": "Index", "obj": base, "idx": ri[1]};
    } else if (((k == "LParen") && (base.tag == "Ident")))     {
      var pos = gx_add(pos, 1);
      var ra = parse_arglist(tokens, pos);
      var pos = ra[0];
      var r2 = eat(tokens, pos, "RParen");
      var pos = r2[0];
      var base = {"tag": "Call", "callee": base, "args": ra[1]};
    } else     {
      var go = false;
    }
  }
  return [pos, base];
}
function parse_arglist(tokens, pos) {
  var args = [];
  while (((!check(tokens, pos, "RParen")) && (!at_end(tokens, pos))))   {
    var ra = parse_expr(tokens, pos);
    var pos = ra[0];
    var args = gx_push(args, ra[1]);
    if (check(tokens, pos, "Comma"))     {
      var pos = gx_add(pos, 1);
    }
  }
  return [pos, args];
}
function parse_primary(tokens, pos) {
  var k = tok_kind(tokens, pos);
  var t = tokens[pos];
  if ((k == "Num"))   {
    var pos = gx_add(pos, 1);
    return [pos, {"tag": "Num", "value": Number(t.value)}];
  }
  if ((k == "Str"))   {
    var pos = gx_add(pos, 1);
    return [pos, parse_interp(t.value)];
  }
  if ((k == "True"))   {
    var pos = gx_add(pos, 1);
    return [pos, {"tag": "Bool", "value": true}];
  }
  if ((k == "False"))   {
    var pos = gx_add(pos, 1);
    return [pos, {"tag": "Bool", "value": false}];
  }
  if ((k == "Null"))   {
    var pos = gx_add(pos, 1);
    return [pos, {"tag": "Null"}];
  }
  if (((k == "Ident") || (k == "Memory")))   {
    var pos = gx_add(pos, 1);
    return [pos, {"tag": "Ident", "name": t.value}];
  }
  if ((k == "LParen"))   {
    var pos = gx_add(pos, 1);
    var re = parse_expr(tokens, pos);
    var pos = re[0];
    var r2 = eat(tokens, pos, "RParen");
    var pos = r2[0];
    return [pos, re[1]];
  }
  if ((k == "LBrack"))   {
    return parse_array(tokens, pos);
  }
  if ((k == "LBrace"))   {
    return parse_object(tokens, pos);
  }
  if ((k != "EOF"))   {
    var pos = gx_add(pos, 1);
  }
  return [pos, {"tag": "Null"}];
}
function parse_array(tokens, pos) {
  var r0 = eat(tokens, pos, "LBrack");
  var pos = r0[0];
  var items = [];
  while (((!check(tokens, pos, "RBrack")) && (!at_end(tokens, pos))))   {
    var ri = parse_expr(tokens, pos);
    var pos = ri[0];
    var items = gx_push(items, ri[1]);
    if (check(tokens, pos, "Comma"))     {
      var pos = gx_add(pos, 1);
    }
  }
  var r1 = eat(tokens, pos, "RBrack");
  var pos = r1[0];
  return [pos, {"tag": "Array", "items": items}];
}
function parse_object(tokens, pos) {
  var r0 = eat(tokens, pos, "LBrace");
  var pos = r0[0];
  var pairs = [];
  while (((!check(tokens, pos, "RBrace")) && (!at_end(tokens, pos))))   {
    var key_tok = tokens[pos];
    var pos = gx_add(pos, 1);
    var key = key_tok.value;
    var r1 = eat(tokens, pos, "Colon");
    var pos = r1[0];
    var rv = parse_expr(tokens, pos);
    var pos = rv[0];
    var pairs = gx_push(pairs, {"key": key, "value": rv[1]});
    if (check(tokens, pos, "Comma"))     {
      var pos = gx_add(pos, 1);
    }
  }
  var r2 = eat(tokens, pos, "RBrace");
  var pos = r2[0];
  return [pos, {"tag": "Object", "pairs": pairs}];
}
function parse_interp(s) {
  var LBRACE = String.fromCharCode(123);
  var RBRACE = String.fromCharCode(125);
  var parts = [];
  var i = 0;
  var n = gx_len(s);
  var buf = "";
  while ((i < n))   {
    var c = s[i];
    if ((c == LBRACE))     {
      if ((gx_len(buf) > 0))       {
        var parts = gx_push(parts, {"tag": "Lit", "v": buf});
        var buf = "";
      }
      var i = gx_add(i, 1);
      var inner = "";
      var depth = 1;
      while (((i < n) && (depth > 0)))       {
        var ch = s[i];
        if ((ch == LBRACE))         {
          var depth = gx_add(depth, 1);
        }
        if ((ch == RBRACE))         {
          var depth = (depth - 1);
        }
        if ((depth > 0))         {
          var inner = gx_add(inner, ch);
        }
        var i = gx_add(i, 1);
      }
      var inner_tokens = tokenize(inner);
      var r = parse_expr(inner_tokens, 0);
      var parts = gx_push(parts, {"tag": "Expr", "e": r[1]});
    } else     {
      var buf = gx_add(buf, c);
      var i = gx_add(i, 1);
    }
  }
  if ((gx_len(buf) > 0))   {
    var parts = gx_push(parts, {"tag": "Lit", "v": buf});
  }
  if (((gx_len(parts) == 1) && (parts[0].tag == "Lit")))   {
    return {"tag": "Str", "value": parts[0].v};
  }
  return {"tag": "Interp", "parts": parts};
}
function sig_return(v) {
  return {"signal": "return", "value": v};
}
function sig_break() {
  return {"signal": "break", "value": null};
}
function sig_continue() {
  return {"signal": "continue", "value": null};
}
function sig_error(msg) {
  return {"signal": "error", "value": msg};
}
function is_sig(s) {
  return (s != null);
}
function eval_program(ast) {
  var env = {};
  var fns = {};
  var i = 0;
  while ((i < gx_len(ast.stmts)))   {
    var s = ast.stmts[i];
    if ((s.tag == "FuncDef"))     {
      var fns = gx_set_key(fns, s.name, {"params": s.params, "body": s.body});
    }
    var i = gx_add(i, 1);
  }
  var i = 0;
  while ((i < gx_len(ast.stmts)))   {
    var s = ast.stmts[i];
    if ((s.tag != "FuncDef"))     {
      var r = eval_stmt(s, env, fns);
      var env = r[0];
      var sig = r[1];
      if ((is_sig(sig) && (sig.signal == "error")))       {
        console.log(gx_str(gx_add("Runtime error: ", sig.value)));
        return;
      }
    }
    var i = gx_add(i, 1);
  }
}
function eval_stmt(s, env, fns) {
  var tag = s.tag;
  if ((tag == "Assign"))   {
    var val = eval_expr(s.value, env, fns);
    var env = do_assign(s.target, val, env);
    return [env, null];
  }
  if ((tag == "PlusEq"))   {
    var cur = eval_expr(s.target, env, fns);
    var rhs = eval_expr(s.value, env, fns);
    var env = do_assign(s.target, eval_add(cur, rhs), env);
    return [env, null];
  }
  if ((tag == "MinusEq"))   {
    var cur = eval_expr(s.target, env, fns);
    var rhs = eval_expr(s.value, env, fns);
    var env = do_assign(s.target, (cur - rhs), env);
    return [env, null];
  }
  if ((tag == "MulEq"))   {
    var cur = eval_expr(s.target, env, fns);
    var rhs = eval_expr(s.value, env, fns);
    var env = do_assign(s.target, (cur * rhs), env);
    return [env, null];
  }
  if ((tag == "DivEq"))   {
    var cur = eval_expr(s.target, env, fns);
    var rhs = eval_expr(s.value, env, fns);
    if ((rhs == 0))     {
      return [env, sig_error("Division by zero")];
    }
    var env = do_assign(s.target, (cur / rhs), env);
    return [env, null];
  }
  if ((tag == "If"))   {
    var i = 0;
    while ((i < gx_len(s.branches)))     {
      var br = s.branches[i];
      var cond = eval_expr(br.cond, env, fns);
      if (is_truthy(cond))       {
        var r = eval_block(br.body, env, fns);
        var env = r[0];
        if (is_sig(r[1]))         {
          return [env, r[1]];
        }
        return [env, null];
      }
      var i = gx_add(i, 1);
    }
    if ((s.else_body != null))     {
      var r = eval_block(s.else_body, env, fns);
      var env = r[0];
      if (is_sig(r[1]))       {
        return [env, r[1]];
      }
    }
    return [env, null];
  }
  if ((tag == "While"))   {
    var guard = 0;
    while (true)     {
      var guard = gx_add(guard, 1);
      if ((guard > 100000))       {
        return [env, sig_error("While loop exceeded 100000 iterations")];
      }
      var cond = eval_expr(s.cond, env, fns);
      if ((!is_truthy(cond)))       {
        break;
      }
      var r = eval_block(s.body, env, fns);
      var env = r[0];
      var sig = r[1];
      if (is_sig(sig))       {
        if ((sig.signal == "break"))         {
          break;
        }
        if ((sig.signal == "continue"))         {
          continue;
        }
        return [env, sig];
      }
    }
    return [env, null];
  }
  if ((tag == "For"))   {
    var iter = eval_expr(s.iter, env, fns);
    var i = 0;
    while ((i < gx_len(iter)))     {
      var env = gx_set_key(env, s.var, iter[i]);
      var r = eval_block(s.body, env, fns);
      var env = r[0];
      var sig = r[1];
      if (is_sig(sig))       {
        if ((sig.signal == "break"))         {
          break;
        }
        if ((sig.signal == "continue"))         {
          var i = gx_add(i, 1);
          continue;
        }
        return [env, sig];
      }
      var i = gx_add(i, 1);
    }
    return [env, null];
  }
  if ((tag == "Return"))   {
    if ((s.value == null))     {
      return [env, sig_return(null)];
    }
    var val = eval_expr(s.value, env, fns);
    return [env, sig_return(val)];
  }
  if ((tag == "Break"))   {
    return [env, sig_break()];
  }
  if ((tag == "Continue"))   {
    return [env, sig_continue()];
  }
  if (((tag == "Log") || (tag == "Say")))   {
    var val = eval_expr(s.value, env, fns);
    console.log(gx_str(gx_to_string(val)));
    return [env, null];
  }
  if ((tag == "Assert"))   {
    var cond = eval_expr(s.cond, env, fns);
    if ((!is_truthy(cond)))     {
      var msg = "Assertion failed";
      if ((s.msg != null))       {
        var msg = gx_to_string(eval_expr(s.msg, env, fns));
      }
      return [env, sig_error(msg)];
    }
    return [env, null];
  }
  if ((tag == "TryCatch"))   {
    var r = eval_block(s.try_body, env, fns);
    var env = r[0];
    var sig = r[1];
    if ((is_sig(sig) && (sig.signal == "error")))     {
      var env = gx_set_key(env, s.catch_var, sig.value);
      var r2 = eval_block(s.catch_body, env, fns);
      var env = r2[0];
      if (is_sig(r2[1]))       {
        return [env, r2[1]];
      }
      return [env, null];
    }
    if (is_sig(sig))     {
      return [env, sig];
    }
    return [env, null];
  }
  if ((tag == "FuncDef"))   {
    var fns = gx_set_key(fns, s.name, {"params": s.params, "body": s.body});
    return [env, null];
  }
  if ((tag == "ExprStmt"))   {
    eval_expr(s.expr, env, fns);
    return [env, null];
  }
  return [env, sig_error(gx_add("Unknown statement: ", tag))];
}
function eval_block(stmts, env, fns) {
  var i = 0;
  while ((i < gx_len(stmts)))   {
    var r = eval_stmt(stmts[i], env, fns);
    var env = r[0];
    if (is_sig(r[1]))     {
      return [env, r[1]];
    }
    var i = gx_add(i, 1);
  }
  return [env, null];
}
function do_assign(target, value, env) {
  if ((target.tag == "Ident"))   {
    return gx_set_key(env, target.name, value);
  }
  if ((target.tag == "Field"))   {
    var obj_name = target.obj.name;
    var obj = env[obj_name];
    if ((obj == null))     {
      var obj = {};
    }
    var obj = gx_set_key(obj, target.field, value);
    return gx_set_key(env, obj_name, obj);
  }
  if ((target.tag == "Index"))   {
    var obj_name = target.obj.name;
    var obj = env[obj_name];
    var idx = eval_expr(target.idx, env, {});
    if ((obj == null))     {
      var obj = {};
    }
    obj[idx] = value;
    return gx_set_key(env, obj_name, obj);
  }
  return env;
}
function eval_expr(node, env, fns) {
  var tag = node.tag;
  if ((tag == "Num"))   {
    return node.value;
  }
  if ((tag == "Str"))   {
    return node.value;
  }
  if ((tag == "Bool"))   {
    return node.value;
  }
  if ((tag == "Null"))   {
    return null;
  }
  if ((tag == "Ident"))   {
    var v = env[node.name];
    return v;
  }
  if ((tag == "Interp"))   {
    var buf = "";
    var i = 0;
    while ((i < gx_len(node.parts)))     {
      var p = node.parts[i];
      if ((p.tag == "Lit"))       {
        var buf = gx_add(buf, p.v);
      } else       {
        var v = eval_expr(p.e, env, fns);
        var buf = gx_add(buf, gx_to_string(v));
      }
      var i = gx_add(i, 1);
    }
    return buf;
  }
  if ((tag == "Array"))   {
    var arr = [];
    var i = 0;
    while ((i < gx_len(node.items)))     {
      var arr = gx_push(arr, eval_expr(node.items[i], env, fns));
      var i = gx_add(i, 1);
    }
    return arr;
  }
  if ((tag == "Object"))   {
    var obj = {};
    var i = 0;
    while ((i < gx_len(node.pairs)))     {
      var pair = node.pairs[i];
      var val = eval_expr(pair.value, env, fns);
      var obj = gx_set_key(obj, pair.key, val);
      var i = gx_add(i, 1);
    }
    return obj;
  }
  if ((tag == "Field"))   {
    var obj = eval_expr(node.obj, env, fns);
    if ((obj == null))     {
      return null;
    }
    return obj[node.field];
  }
  if ((tag == "Index"))   {
    var obj = eval_expr(node.obj, env, fns);
    var idx = eval_expr(node.idx, env, fns);
    if ((obj == null))     {
      return null;
    }
    return obj[idx];
  }
  if ((tag == "NullCoal"))   {
    var lv = eval_expr(node.left, env, fns);
    if ((lv != null))     {
      return lv;
    }
    return eval_expr(node.right, env, fns);
  }
  if ((tag == "Unary"))   {
    var v = eval_expr(node.expr, env, fns);
    if ((node.op == "not"))     {
      return (!is_truthy(v));
    }
    if ((node.op == "-"))     {
      return (0 - v);
    }
    return null;
  }
  if ((tag == "BinOp"))   {
    var op = node.op;
    if ((op == "and"))     {
      var lv = eval_expr(node.left, env, fns);
      if ((!is_truthy(lv)))       {
        return false;
      }
      return is_truthy(eval_expr(node.right, env, fns));
    }
    if ((op == "or"))     {
      var lv = eval_expr(node.left, env, fns);
      if (is_truthy(lv))       {
        return true;
      }
      return is_truthy(eval_expr(node.right, env, fns));
    }
    var lv = eval_expr(node.left, env, fns);
    var rv = eval_expr(node.right, env, fns);
    if ((op == "+"))     {
      return eval_add(lv, rv);
    }
    if ((op == "-"))     {
      return (lv - rv);
    }
    if ((op == "*"))     {
      return (lv * rv);
    }
    if ((op == "/"))     {
      return (lv / rv);
    }
    if ((op == "%"))     {
      return (lv % rv);
    }
    if ((op == "=="))     {
      return (lv == rv);
    }
    if ((op == "!="))     {
      return (lv != rv);
    }
    if ((op == "<"))     {
      return (lv < rv);
    }
    if ((op == ">"))     {
      return (lv > rv);
    }
    if ((op == "<="))     {
      return (lv <= rv);
    }
    if ((op == ">="))     {
      return (lv >= rv);
    }
    return null;
  }
  if ((tag == "Call"))   {
    return eval_call(node, env, fns);
  }
  if ((tag == "MethodCall"))   {
    return eval_method(node, env, fns);
  }
  return null;
}
function eval_call(node, env, fns) {
  var fname = node.callee.name;
  var args = eval_arglist(node.args, env, fns);
  if ((((fname == "log") || (fname == "print")) || (fname == "say")))   {
    console.log(gx_str(gx_to_string(args[0])));
    return null;
  }
  if ((fname == "len"))   {
    return gx_len(args[0]);
  }
  if ((fname == "to_string"))   {
    return gx_to_string(args[0]);
  }
  if ((fname == "to_number"))   {
    return Number(args[0]);
  }
  if ((fname == "range"))   {
    if ((gx_len(args) == 1))     {
      return gx_range(0, args[0]);
    }
    if ((gx_len(args) == 2))     {
      return gx_range(args[0], args[1]);
    }
    return gx_range(args[0], args[1], args[2]);
  }
  if ((fname == "ord"))   {
    return (args[0]).charCodeAt(0);
  }
  if ((fname == "chr"))   {
    return String.fromCharCode(args[0]);
  }
  if ((fname == "is_digit"))   {
    return gx_is_digit(args[0]);
  }
  if ((fname == "is_alpha"))   {
    return gx_is_alpha(args[0]);
  }
  if ((fname == "is_alnum"))   {
    return gx_is_alnum(args[0]);
  }
  if ((fname == "is_whitespace"))   {
    return gx_is_whitespace(args[0]);
  }
  if ((fname == "floor"))   {
    return Math.floor(args[0]);
  }
  if ((fname == "ceil"))   {
    return Math.ceil(args[0]);
  }
  if ((fname == "abs"))   {
    return Math.abs(args[0]);
  }
  if ((fname == "sqrt"))   {
    return Math.sqrt(args[0]);
  }
  if ((fname == "get_timestamp"))   {
    return Date.now();
  }
  if ((fname == "push"))   {
    return gx_push(args[0], args[1]);
  }
  if ((fname == "pop"))   {
    return ((..._a) => null)([args[0]]);
  }
  if ((fname == "join"))   {
    return args[0].join(args[1]);
  }
  if ((fname == "split"))   {
    return args[0].split(args[1]);
  }
  if ((fname == "trim"))   {
    return args[0].trim();
  }
  if (((fname == "upper") || (fname == "to_upper")))   {
    return args[0].toUpperCase();
  }
  if (((fname == "lower") || (fname == "to_lower")))   {
    return args[0].toLowerCase();
  }
  if ((fname == "contains"))   {
    return args[0].includes(args[1]);
  }
  if ((fname == "starts_with"))   {
    return args[0].startsWith(args[1]);
  }
  if ((fname == "ends_with"))   {
    return args[0].endsWith(args[1]);
  }
  if ((fname == "replace"))   {
    return args[0].replaceAll(args[1], args[2]);
  }
  if ((fname == "set_key"))   {
    return gx_set_key(args[0], args[1], args[2]);
  }
  if ((fname == "keys"))   {
    return Object.keys(args[0]);
  }
  if ((fname == "values"))   {
    return Object.values(args[0]);
  }
  if ((fname == "json_parse"))   {
    return JSON.parse(args[0]);
  }
  if ((fname == "json_stringify"))   {
    return JSON.stringify(args[0]);
  }
  if ((fname == "read_file"))   {
    return read_file(args[0]);
  }
  if ((fname == "write_file"))   {
    write_file(args[0], args[1]);
    return null;
  }
  if ((fname == "file_exists"))   {
    return file_exists(args[0]);
  }
  if ((fname == "env"))   {
    return __builtin_env(args[0]);
  }
  if ((fname == "tokenize"))   {
    return tokenize(args[0]);
  }
  if ((fname == "parse"))   {
    return parse(args[0]);
  }
  var fn_def = fns[fname];
  if ((fn_def != null))   {
    var fn_env = {};
    var i = 0;
    while ((i < gx_len(fn_def.params)))     {
      var val = null;
      if ((i < gx_len(args)))       {
        var val = args[i];
      }
      var fn_env = gx_set_key(fn_env, fn_def.params[i], val);
      var i = gx_add(i, 1);
    }
    var r = eval_block(fn_def.body, fn_env, fns);
    var sig = r[1];
    if ((is_sig(sig) && (sig.signal == "return")))     {
      return sig.value;
    }
    return null;
  }
  console.log(gx_str(gx_add("Unknown function: ", fname)));
  return null;
}
function eval_method(node, env, fns) {
  var obj = eval_expr(node.obj, env, fns);
  var m = node.method;
  var args = eval_arglist(node.args, env, fns);
  if ((m == "push"))   {
    return gx_push(obj, args[0]);
  }
  if ((m == "pop"))   {
    return ((..._a) => null)([obj]);
  }
  if ((m == "first"))   {
    return obj[0];
  }
  if ((m == "last"))   {
    return obj[obj.length - 1];
  }
  if ((m == "join"))   {
    return obj.join(args[0]);
  }
  if ((m == "reverse"))   {
    return [...obj].reverse();
  }
  if ((m == "split"))   {
    return obj.split(args[0]);
  }
  if ((m == "trim"))   {
    return obj.trim();
  }
  if ((m == "to_upper"))   {
    return obj.toUpperCase();
  }
  if ((m == "to_lower"))   {
    return obj.toLowerCase();
  }
  if ((m == "contains"))   {
    return obj.includes(args[0]);
  }
  if ((m == "starts_with"))   {
    return obj.startsWith(args[0]);
  }
  if ((m == "ends_with"))   {
    return obj.endsWith(args[0]);
  }
  if ((m == "replace"))   {
    return obj.replaceAll(args[0], args[1]);
  }
  if (((m == "to_number") || (m == "parse_number")))   {
    return Number(obj);
  }
  if (((m == "has") || (m == "has_key")))   {
    return Object.hasOwn(obj, args[0]);
  }
  if ((m == "keys"))   {
    return Object.keys(obj);
  }
  if ((m == "values"))   {
    return Object.values(obj);
  }
  if (((m == "len") || (m == "length")))   {
    return gx_len(obj);
  }
  if (((m == "slice") || (m == "substring")))   {
    if ((gx_len(args) == 1))     {
      return obj.slice(args[0], gx_len(obj));
    }
    return obj.slice(args[0], args[1]);
  }
  if ((m == "repeat"))   {
    return obj.repeat(args[0]);
  }
  if ((m == "index_of"))   {
    return obj.indexOf(args[0]);
  }
  console.log(gx_str(gx_add("Unknown method: ", m)));
  return null;
}
function eval_arglist(arg_nodes, env, fns) {
  var args = [];
  var i = 0;
  while ((i < gx_len(arg_nodes)))   {
    var args = gx_push(args, eval_expr(arg_nodes[i], env, fns));
    var i = gx_add(i, 1);
  }
  return args;
}
function is_truthy(v) {
  if ((v == null))   {
    return false;
  }
  if ((v == false))   {
    return false;
  }
  if ((v == 0))   {
    return false;
  }
  if ((v == ""))   {
    return false;
  }
  return true;
}
function eval_add(a, b) {
  return gx_add(a, b);
}
function gx_to_string(v) {
  if ((v == null))   {
    return "null";
  }
  if ((v == true))   {
    return "true";
  }
  if ((v == false))   {
    return "false";
  }
  return gx_str(v);
}
function gx_main() {
  var target_file = __builtin_env("GX_FILE");
  if ((target_file == null))   {
    var target_file = "self/test_self.gx";
  }
  if ((!file_exists(target_file)))   {
    console.log(gx_str(gx_add("Error: file not found: ", target_file)));
  } else   {
    var src = read_file(target_file);
    var tokens = tokenize(src);
    var ast = parse(tokens);
    eval_program(ast);
  }
}
gx_main();
