# GX Language Engineering Review — AgentX Production Feedback (2026-07)

Source: AgentX (~6,000 LOC, 34 files, 17 agents, 11 libs, scheduler, live
dashboard, SQLite, production deploy) built end-to-end against GX 0.6.1.
Every finding below was independently verified against the current source
tree (`git diff v0.6.1 -- src/` is empty, so HEAD == the exact release the
report was written against) — file:line citations are real, not inferred
from the report text.

---

## Executive Summary

GX's core execution model — lexer → two front-end parsers → one AST →
tree-walking interpreter — is sound, and the parts of the language built
around *runtime I/O and AI calls* (capability system, `process_run`,
`db_transaction`, `ask`, `gx test`) are genuinely well-designed and match
the report's praise. The damage in this feedback is concentrated almost
entirely in a different place: **program-structure handling** — variable
resolution, function identity, syntax-mode dispatch, and interpolation —
where the interpreter consistently chooses to *degrade silently* rather
than *fail loudly*. That one instinct, applied independently in at least
three unrelated subsystems, is responsible for every "silent corruption"
item in §1 of the feedback. It is also in direct tension with GX's own
stated philosophy ("every AI decision explicit, every AI call logged,
every agent fully debuggable") — the interpreter is transparent about
*AI* behavior and opaque about its *own* behavior.

None of this requires a rewrite. The fixes cluster into a small number of
architectural changes (a real scope chain; loud failure on unresolvable
identifiers; a narrower syntax-mode sniff), most of which are additive or
low-risk, plus one item (§1.1, the formatter) that I could **not**
reproduce against current source and believe was already fixed upstream
before this feedback was written — see the dedicated note below. I'd
push back on treating it as an open release blocker without confirming
the reporter's exact binary version first.

---

## Root Causes

### RC1 — No lexical scope chain; declarative top-level constructs live in side-tables

`Env` (`src/interpreter/mod.rs:266-304`) is a flat `HashMap<String,
Value>` with **no parent-scope pointer**. `call_user_function` /
`call_user_function_propagating` (`mod.rs:4302-4357`) always start a call
from `Env::new()` — an empty scope holding only the call's own
parameters. There is no mechanism for a function body to see anything
defined outside it, by design, *except* for two ad hoc side-channels that
only specific call sites know to consult:

- Top-level bare assignments (`NAME = value` at file scope) get copied
  into `self.global_vars: HashMap<String, Value>` (`mod.rs:394`), a store
  entirely separate from `Env`. The **only** reader of `global_vars` is
  `run_helper`, which injects it into an *agent's* env as ordinary locals
  (`mod.rs:1562-1567`). Plain functions never receive this injection.
- `function name() {}` declarations are collected into `self.functions:
  HashMap<String, FunctionDef>` (`mod.rs:310`) — a name→definition table
  outside the value system entirely. It is consulted **only** by
  `eval_call`'s identifier-call special case (`mod.rs:3837-3852`, the
  `self.functions.get(name)` branch at 3848), which fires only for
  `name(...)` call syntax. Ordinary bare-identifier evaluation
  (`Expr::Ident`, `mod.rs:3301-3327`) never looks at `self.functions` at
  all.

This single decision — "globals and named functions are declarative,
therefore they live outside the normal variable path, and only the call
sites we remembered to wire up can see them" — is the root cause of
**both** 1.3 (top-level const invisible in a function) and 1.5 (a named
function isn't a referenceable value). It also explains why 2.1 (imports
silently drop top-level statements) reads as *natural* rather than
alarming from inside the codebase: top-level statements were never fully
first-class to begin with, so treating them as import-invisible is a
small step, not a special case.

### RC2 — Program-structure errors degrade silently instead of failing loudly

This is the throughline connecting nearly every item in the feedback's
"silent corruption" bucket, and it recurs in **three independently
written subsystems**, which is what makes it a root cause rather than
one bug:

1. **`Env::get`** (`mod.rs:275-277`) returns `Value::Null` for *any*
   unbound identifier. There is no "undefined variable" error anywhere
   in the runtime. This is what makes 1.3 (`MY_CONST` inside a function)
   and 1.5 (`say named`) *silent* rather than loud — the lookup doesn't
   fail, it just quietly hands back null. It's also what makes 3.2
   (`write "x"`) a silent no-op: the parser (see RC6) turns it into two
   separate expression-statements, `Expr::Ident("write")` evaluates to
   `Null` via this exact path, and both results are discarded.
2. **`parse_interpolated`** (`src/parser.rs:2372-2459`) tokenizes and
   parses the contents of every `{...}` inside a string literal using a
   full sub-lexer/sub-parser pass. When that pass fails — e.g. because
   the expression contains a single-quoted string, which the GX lexer
   doesn't support at all (`read_string` only fires on `"`,
   `lexer.rs:409`; any `'` hits the catch-all "unexpected character"
   error, `lexer.rs:573`) — the function doesn't propagate the error. It
   falls back to `InterpolatedPart::Literal(format!("{{{}}}", expr_src))`
   (`parser.rs:2439`), silently re-emitting the original `{...}` text as
   a string literal. This is 2.5.
3. **`indent_parser`'s top-level loop** (`indent_parser.rs:685-729`)
   silently skips (`idx += 1`, line 726-728) any top-level line it
   doesn't recognize, rather than raising a parse error. Combined with
   RC3 below, this is the actual mechanism behind 1.4.

Three different people (or the same person on three different days)
independently reached for "if this doesn't parse/resolve the way I
expect, quietly do something plausible" instead of "fail immediately
with a specific error." That instinct is defensible for *AI output
handling* (don't crash a pipeline because a model returned an unexpected
shape) — it is actively dangerous for *program structure* (identifiers,
tokens, top-level constructs), where the cost of silent degradation is a
multi-hour debugging session tracing a `null` back through several
unrelated call sites, exactly as the report describes.

### RC3 — Syntax-mode detection is a whole-file substring scan, not a structural check

`is_indent_syntax` (`indent_parser.rs:61-95`) decides whether an entire
file is parsed as brace-syntax or indentation-syntax by scanning **every
line of the file** for one of four patterns: an unquoted `agent `/`helper
` prefix, a bare `plan:`/`execute:`/`remember:`/`communicate:` line, an
`on ...:` line, or a bare `agent`/`helper` token. Any single line
anywhere in an otherwise-normal brace-syntax file that happens to match
— not just at the top of the file — reroutes the **entire file** to a
different grammar. This is the actual mechanism behind 1.4: `agent =
"*"` inside a function body matches indicator 1 (`lower.starts_with("agent
")`, the value doesn't start with `"` or `{`), the whole file gets parsed
by `indent_parser::parse`, and every construct that parser's minimal
top-level grammar doesn't recognize — including the entire `function
f() { ... }` and the `f()` call — is dropped per RC2's third instance.
The user's own diagnosis (parse error on `agent` as identifier) was
half right; the actual failure is one level up, in mode dispatch, and is
*worse* than a parse error because it's silent.

This is a landmine independent of the reported repro: indicator 3 (`on
...:`) is an extremely common English-language shape. Any brace-syntax
file in an agent/scheduler-flavored codebase — exactly AgentX's domain —
that has a comment not using `//`, or a multi-line string, containing
something shaped like `On error:` would trigger the same silent
misroute. `is_indent_syntax` is used identically by `gx run`
(`toolchain.rs:1076`), `gx check` (`toolchain.rs:1535`), and `gx fmt`
(`toolchain.rs:819`) — the same false positive corrupts all three
commands' behavior, not just execution.

### RC4 — No canonical representation shared across sibling builtins

Confirmed for the datetime module specifically
(`src/interpreter/builtins_datetime.rs`): every consumer accepts either a
number or an ISO string via `to_timestamp` (`:215-231`, and the module's
own doc comment says as much), but on the *output* side, `date_now()`
alone returns `Value::Str`; `date_parse`, `date_add`, `date_from_parts`,
and the numeric fields of `date_parts` all return `Value::Number`. The
numeric Unix timestamp is the de facto canonical internal type
everywhere except the one function most users will treat as their
starting point. This is not a single-function bug (1.2) — it's a
module-wide inconsistency. I have not audited every other builtin module
for the same pattern (out of scope for this pass), but the shape of the
bug — "accepts multiple representations, silently normalizes to a
different one than expected on output" — is worth a targeted audit
before the next release, not an assumption that it's isolated to dates.

### RC5 — CLI argument handling is hand-rolled per-flag, not structured for N-ary commands

`main.rs` has no argument-parsing library; subcommand args are read via
direct indexing (`require_arg(&args, 2, ...)`, `main.rs:830-835`) and
flag presence via `.contains(&"--flag".to_string())` scans. This explains
two independent, otherwise-unrelated bugs as one root cause: `gx check
file1.gx file2.gx` only reads `args[2]` and never looks at `args[3..]`
(`main.rs:152-155`, `cmd_check` is typed `&str` not `&[String]`) — 4's
`gx check` bug; and `gx run file.gx foo bar` never collects `args[3..]`
into anything the interpreter can see — no `argv()`/`script_args()`
builtin exists (2.3). Both are gaps in the same under-built layer, not
independent design decisions.

### RC6 (minor, compounding) — No statement separator required between statements

`parse_stmts` (`parser.rs:996-1007`) only calls `skip_newlines` between
statements — it never requires one. This means `write "x"` parses
without error as **two** adjacent `Stmt::Expr` statements
(`Expr::Ident("write")` then `Expr::Str("x")`) rather than one malformed
statement. Combined with RC2 (unbound idents evaluate to `Null` instead
of erroring), a typo that in almost any other language would be a syntax
error becomes a silent double no-op instead. This is why 3.2 fails
*silently* rather than *loudly* — the grammar gap alone would just cause
a slightly different parse, not silence; it's RC6 + RC2 together that
produce the reported symptom.

---

## Issue Classification (by subsystem)

### Runtime / Scope Resolution
| # | Issue | Classification | Severity |
|---|---|---|---|
| 1.3 | Top-level `NAME = value` invisible inside functions | Runtime bug (RC1) | **Release blocker** |
| 1.5 | `function name(){}` not a referenceable value | Runtime/API design flaw (RC1) | **Critical** |
| 3.2 (part) | `write "x"` silent no-op | Runtime bug (RC2+RC6) | High |

**Why 1.3 is a release blocker, not just critical:** it silently produces
wrong *data*, not just wrong control flow — the report's own example (a
`TARGET_PATHS` array silently empty for the entire development period,
with zero errors) shows this can hollow out a program's actual behavior
while `gx check`, `gx test` (if the test itself references the same
broken global), and a normal run all report success. There is no
diagnostic surface in the current toolchain that would ever catch this
class of bug short of noticing wrong output by hand.

### Parser / Syntax-Mode Dispatch
| # | Issue | Classification | Severity |
|---|---|---|---|
| 1.4 | `agent` as identifier silently produces an empty program | Parser bug (RC3+RC2) | **Release blocker** |
| — | (unreported) any brace file with an `On ...:`-shaped line anywhere | Parser bug (RC3) | Critical (latent) |
| 2.5 | Interpolated method call with a quoted-string arg silently unparsed | Parser bug (RC2, single-quote gap) | High |

**Why 1.4 is a release blocker:** identical mechanism to 1.3 — the
program silently becomes a no-op (exit 0, no output, no error) rather
than failing. Worse, it's not scoped to the reported trigger (`agent` as
a variable) — RC3 shows the misroute condition is broader than the
report itself realized, which raises the practical severity above what
a single repro suggests.

### Lexer
| # | Issue | Classification | Severity |
|---|---|---|---|
| 3.1 | `&&`/`||` documented, not implemented | Documentation/implementation mismatch | Medium (loud parse error, not silent) |
| 2.4 | `"{{"..."}}"` collapses `{{` but not the standalone `}}` case | Implementation bug, one-line guard (`parser.rs:2372-2374`) | Medium |

### Formatter
| # | Issue | Classification | Severity |
|---|---|---|---|
| 1.1 | Identifier truncation before `}` | **Could not reproduce** — see note below | Needs reproduction before triage |
| 4 (style) | Heavy padding around every paren/bracket | Style/config, not correctness | Low |

**Formatter note — do not treat as confirmed without further evidence.**
`format_source` (`toolchain.rs:814-872`) is a genuine token-stream
reprinter (`Lexer::tokenize()` → per-token emission via `token_to_str`,
`toolchain.rs:874-1035`) — there is no line-slicing, no cross-line
lookahead, and no code path that could plausibly drop one trailing
character of an identifier based on what follows on the next line. I
rebuilt the release binary from current HEAD and ran the exact repro
from the feedback plus eight variations (CRLF, tabs, trailing whitespace,
nested blocks, `for`/`while`, bare `return`) — none reproduced any
truncation; every identifier round-tripped byte-exact. `token_to_str`'s
match is exhaustive over `TokenKind` (`toolchain.rs:1033`, no `_`
wildcard), with a code comment and a regression test
(`format_source_never_silently_drops_a_keyword`, `toolchain.rs:2340`)
specifically guarding against a **related but distinct** bug class —
whole-token silent deletion via a non-exhaustive match arm — that *did*
exist and *was* fixed, in commit `44905c9` ("production runtime
completion + Phase 2 hardening"), dated 2026-07-11, one day before the
v0.6.1 tag. That fix addressed whole-keyword loss, not partial-identifier
corruption, so it doesn't fully explain the reported symptom either way.

My working hypothesis: the reporter's binary predates `44905c9` (e.g. a
cached `npm install -g gxlang` from before the fix propagated) and hit
the whole-keyword-loss bug, which — depending on exactly which keyword
and identifier collided in their real files — could plausibly *look*
like single-character truncation when eyeballing a diff. I'd want the
reporter's exact `gx --version` output and, ideally, one of the nine
actually-corrupted files before writing new formatter code. **Do not
skip this verification step** — writing a fix for a mechanism that
doesn't exist in the code wastes effort and won't close the reporter's
actual gap if it's a stale-binary issue. Regardless of root cause, I'd
still add a dedicated identifier-round-trip property test (format many
synthetic "ident immediately before `}`" cases and assert byte-exact
`Ident` token content survives) as a permanent CI gate — formatter trust
is close to existential for this feature, and the current test suite
covers keyword-loss and string-escaping but not this specific class.

### Type System / Builtins
| # | Issue | Classification | Severity |
|---|---|---|---|
| 1.2 | `date_add` returns number; `date_now` returns string | API design flaw (RC4) | **Critical** |

Silent, not a crash: storing the numeric result into a column that
otherwise holds ISO strings and comparing against `date_now()` produces
a string comparison between a numeric-looking string and an ISO string,
which is *always true* — a scheduled action fires immediately instead
of after N days. This is data-corruption-adjacent (wrong business
behavior with no error surface) but I'm classifying it one notch below
1.3/1.4 because it requires the specific pattern of writing the result
into a string-typed store, rather than corrupting the language's own
control flow.

### Module / Import System
| # | Issue | Classification | Severity |
|---|---|---|---|
| 2.1 | `import` only merges functions/agents/tools, drops top-level statements | Intended behavior, needs documentation (and is a symptom of RC1) | Medium |

Confirmed at `resolve_file_imports` (`mod.rs:1138-1282`): only
`sub.functions`/`sub.helpers`/`sub.tools` are merged; `sub.top_level_stmts`
is never referenced anywhere outside the entry-file's own execution.
Defensible as a design choice (no import-time side effects) but
currently undocumented and, per RC1, not really a *decision* so much as
a *consequence* of top-level statements never having a real home in the
scope model to begin with.

### Capability / Sandbox
| # | Issue | Classification | Severity |
|---|---|---|---|
| 2.2 | Sandbox root fixed to entry script's directory for the whole process | API design flaw (granularity gap) | Medium |

Confirmed: `main.rs:404-423` computes `script_dir` once from the
entry-point path and sets `FilesystemAccess::Sandboxed(script_dir)` once;
`Capabilities::resolve_path`/`sandbox_dir` (`capability.rs:401-517`) only
ever consult this single process-lifetime value — there is no
call-stack-aware re-derivation for code running from an imported file in
a different directory. Not a vulnerability (it's stricter than
necessary, not looser), but forces the project-flattening workaround the
report describes.

### CLI
| # | Issue | Classification | Severity |
|---|---|---|---|
| 2.3 | No `argv()`/script-args mechanism | Implementation gap (RC5) | Medium |
| 4 | `gx check file1 file2 ...` silently checks only the first | Implementation bug (RC5) | Low-Medium (silent, but low blast radius — CI users notice missing coverage eventually) |

### Diagnostics
No dedicated diagnostics-subsystem bug was found, but nearly every item
in RC2 and RC3 is, at its core, a **missing diagnostic**: `gx check`
currently has no rule for "unbound identifier," "discarded non-call
expression statement," "single-quoted string inside interpolation," or
"file matched the indent-syntax heuristic on a non-top-level line." All
four are statically detectable and would have caught the reported bugs
before runtime. This is a gap worth its own workstream (see Recommended
Fix Order).

### Documentation / Release Process
| # | Issue | Classification | Severity |
|---|---|---|---|
| 3.1 | `&&`/`||` in docs, not lexer | Documentation mismatch | Medium |
| 3.2 | `write` shown alongside `say`/`log` implying shared syntax | Documentation gap | Low-Medium |
| 3.3 | `install.sh` installs stale v0.1.0, fails self-test | Release-process bug, not a language bug | **Critical** (first-run experience) |
| 4 | No ternary operator | Intended, needs one doc line | Low |

3.3 wasn't code-investigated in this pass (it's a deployed static script,
not part of `src/`) — flagging it as release-process scope, likely
related to how `install.sh`'s version pin is (or isn't) updated by the
existing release automation (`release.yml`, per prior GX release-process
notes). Recommend it fetch "latest" from the GitHub Releases API rather
than embedding a hardcoded version, so it can never drift behind
`crates.io`/`npm` again.

---

## Recommended Fix Order

The goal stated in the brief — fewest architectural changes, most bugs
closed — points at a clear order:

**Tier 0 — narrow, independent, low-risk (ship first, no design debate needed):**
1. `is_indent_syntax` — restrict the four positive indicators to lines
   that are structurally top-level (indent 0, outside any string/comment,
   before the first `{`/`function`/statement that establishes brace
   syntax) rather than scanning the whole file. Fixes 1.4 and the
   broader latent RC3 landmine in one change. This alone is probably the
   single highest-value fix in the whole review relative to its size.
2. `indent_parser`'s top-level loop: replace the silent `idx += 1` skip
   with a parse error naming the unrecognized line. Defense-in-depth for
   any future RC3-shaped miss.
3. `parse_interpolated`'s `{{`/`}}` guard — add the missing `}`
   check (2.4). One line.
4. Lexer: add `&&`/`||` as aliases for `and`/`or` (3.1). Two match arms,
   zero grammar/AST changes.
5. `gx check`: loop over all positional file args instead of just the
   first (4).
6. `parse_interpolated`: stop silently swallowing the sub-lexer/parser
   error on fallback (`parser.rs:2436-2439`) — surface it as a compile
   error with the original span. This alone converts 2.5 from a silent
   miscompile into a loud, fixable error even before deciding whether to
   add single-quote string support.
7. Formatter: confirm reproduction status with the reporter (exact `gx
   --version`) before writing any fix for 1.1; add the identifier
   round-trip property test regardless.

**Tier 1 — the scope-chain redesign (RC1, addresses 1.3 + 1.5 together):**
Give `Env` a real parent-scope link (or, equivalently, fold
`global_vars` and `functions` into one enclosing scope that every call
frame chains to). This is the "cleanest long-term solution" the brief
asked for, and it's a genuine two-birds-one-stone change:
- Top-level bare assignments become ordinary bindings in an enclosing
  scope, visible from any function by normal lookup — no more
  `global_vars` side-channel, no more `run_helper`-only injection.
- `function name(){}` declarations can be represented as ordinary
  `Value::Closure` bindings in that same enclosing scope instead of a
  separate `self.functions` table, unifying them with `fn(){}` — a bare
  reference to `named` then naturally returns the same closure value
  `named(...)` would call, exactly matching `fn(){}`'s behavior and
  closing 1.5 without a special case.

This is a moderate-to-large change (touches `run_program`,
`call_user_function[_propagating]`, `Expr::Ident` lookup, and however
`run_helper`'s existing memory-injection interacts with a new enclosing
scope) and needs its own design pass before implementation — I'd treat
it as its own subsystem session per the brief's "one subsystem at a
time" plan, not something to fold into the Tier 0 patch set.

**Tier 2 — RC2's biggest lever, needs a migration story:**
Make `Env::get` (or a wrapper around it used specifically for bare
`Expr::Ident` evaluation, as opposed to optional-chaining-style property
access which should keep null-safe semantics) raise a runtime error for
a genuinely unbound top-level identifier, instead of returning `Value::Null`
unconditionally. This is the single change that would have caught 1.3,
1.5, and 3.2 immediately, at the exact call site, instead of downstream.
It's also the riskiest compat-wise (see below) — I'd sequence it *after*
Tier 1, since Tier 1 removes the two biggest legitimate reasons an
identifier "should" resolve to something a flat scope can't currently
find (top-level consts, named functions), shrinking the set of programs
that would newly start erroring.

**Tier 3 — design decisions requiring your sign-off before implementation:**
- Date builtins' canonical representation (1.2 / RC4) — recommend ISO
  string as canonical everywhere except an explicitly-numeric
  `date_timestamp()` escape hatch, for consistency with "auditable,
  human-legible by default." This is a breaking change for anything
  currently consuming `date_add`'s numeric return; needs a deprecation
  window.
- `argv()`/`script_args()` builtin (2.3) — purely additive, no compat
  question, just needs an interface decision (flat array vs. parsed
  flags+positionals).
- A project-root-scoped capability level between "entry script's own
  directory" and "no sandbox" (2.2) — purely additive.
- Whether single-quoted strings should be supported as a second string
  syntax, or whether `gx check` should instead flag them as a clear
  error wherever they currently silently misparse (2.5's second half).

---

## Backward Compatibility Analysis

| Change | Compatibility | Migration |
|---|---|---|
| Narrow `is_indent_syntax` to top-level-only lines | **Compatible** | Any file currently misrouted was already broken (silently emptied); narrowing the check can only make previously-broken files start working correctly. No currently-*working* program can regress, since the check only gets *stricter* about what counts as a positive indicator. |
| Indent-parser: error instead of silently skip unrecognized top-level lines | **Compatible** | Same reasoning — a line that was silently dropped was never contributing correct behavior. Only risk: a real user who was (knowingly or not) relying on the silent-skip as an ad hoc "comment out with any syntax" mechanism; vanishingly unlikely and not a supported pattern. |
| `{{`/`}}` guard fix | **Compatible** | Only changes output for strings containing a literal `}}` with no `{`, which currently round-trip *wrong*; any code depending on the current wrong output was already broken. |
| `&&`/`||` lexer support | **Compatible** | Purely additive tokens; `and`/`or` keep working unchanged. |
| `gx check` multi-file loop | **Compatible** | Strictly more coverage for the same invocation; nothing currently passing can start failing unless it has real errors in the previously-ignored files (which is the point). |
| Surface `parse_interpolated`'s swallowed error | **Partially compatible** | Any interpolation that currently silently no-ops (prints the literal `{...}` text) will now be a compile error instead. This is a **behavior change for currently-"working"** (i.e., silently wrong) programs — it will break any `.gx` file that has this exact latent bug today, by turning a silent wrong-output bug into a build failure. That's the intended outcome, but it should ship with a clear error message and probably a changelog callout, since it's the one Tier-0 item that can newly fail a `gx check` that previously passed. |
| Tier 1 scope-chain unification | **Partially compatible** | Programs that currently rely on a top-level const being *invisible* inside a function (i.e., code that assigns the same name locally inside a function expecting shadowing, unaware a same-named global exists) would begin observing the global instead of `null`. In a tree-walking interpreter with proper shadowing rules (local binding always wins over enclosing scope) this should be rare and is standard scoping semantics in every comparable language — but it's a real semantic change, not purely additive, and deserves a `gx check` lint pass across a representative corpus (AgentX itself would be a good test corpus) before shipping. Making a previously-named `function` referenceable as a bare value is purely additive (nothing currently depends on `say named` printing `null`). |
| `Env::get` errors on unbound identifiers | **Breaking**, by design | This is the one change in this review that cannot be made silently compatible — its entire value is that programs which currently run to completion with wrong/null data will now fail fast instead. Recommended migration: ship it first as a `gx check` *warning* (static, whole-program reachability analysis for identifiers with no possible binding), let it run for a release or two so users can clean up flagged code, then flip it to a hard runtime error in a subsequent major version once Tier 1 has shrunk the false-positive surface (globals and named functions will otherwise trip this constantly pre-Tier-1). Do not ship it as a silent runtime behavior change without the warning period — that would reproduce exactly the "silent corruption on upgrade" failure mode this whole review is about, just moved to compile time. |
| Date builtins → ISO-string-canonical | **Breaking** | Any code storing/comparing `date_add`'s current numeric output changes behavior. Migration: introduce the string-returning behavior behind the existing `date_add` name only in a major version bump, with a `gx check` lint in the prior minor version that flags `date_add(...)` results being compared against or concatenated with string-typed values (a static, detectable pattern) to help users find call sites in advance. |
| `argv()` builtin, project-root sandbox level | **Compatible** | Strictly additive; no existing program references either. |
| Formatter padding style change | **Compatible in effect, but a one-time diff shock** | Doesn't change program *behavior*, only `gx fmt`'s output shape — every previously-formatted file will show as "changed" on the next `gx fmt` run. Fine to ship, but call it out in the changelog so a CI format-check step doesn't surprise anyone. |

---

## Language Design Recommendations

1. **Codify "structural errors are always loud" as a language invariant,
   separate from "runtime data is handled gracefully."** GX already
   nails the second half — `ask` with confidence scoring, graceful
   fallback across providers, capability-denial messages that name
   exactly what's missing. RC2 shows the same graceful-degradation
   instinct leaking into the wrong layer: identifiers, tokens, and
   top-level program structure. These are two different value systems
   and should be developed with different defaults — degrade gracefully
   on *AI/external data*, fail immediately on *program structure*. This
   single principle, if written down and enforced in code review for
   this codebase going forward, would have prevented every item in
   feedback §1.

2. **Grow `gx check` into the primary defense against this whole class of
   bug**, not just a syntax linter. Concretely: unbound-identifier
   reachability analysis (catches 1.3/1.5 statically, pre-runtime, before
   Tier 2 even ships as a hard error), a lint for discarded non-call
   expression statements (catches 3.2-shaped typos), a lint for
   single-quoted strings appearing inside `{...}` interpolation (catches
   2.5 statically, as the report itself suggested), and a lint flagging
   any line matching an `is_indent_syntax` positive indicator outside
   the true top of a brace-syntax file (defense-in-depth for RC3 beyond
   the parser-level fix). Every one of these is a compile-time-detectable
   pattern; none require runtime tracing.

3. **Document import semantics as a decision, not an omission**, once
   Tier 1 lands. "Importing a file runs its declarations, not its
   top-level statements — call your own `ensure_schema()`/`init()`
   explicitly" is a genuinely good, deterministic, auditable design
   (no import-time side effects is exactly the kind of "minimal runtime
   magic" GX should keep) — it just needs a home in the Imports section
   of the language reference, with the AgentX-report's own pattern (every
   entry-point calls its own init function) given as the canonical
   example.

4. **Treat formatter trust as a first-class CI invariant, permanently.**
   Independent of whether 1.1 reproduces: a language whose formatter can
   ever silently alter identifier text is worse than having no formatter,
   full stop — that's the report's own framing and it's correct. Add
   (a) the identifier round-trip property test recommended above, (b)
   the existing idempotency test extended to run against every `.gx`
   file the interpreter's own test suite already exercises (dogfooding
   at zero extra cost), and (c) a policy that any formatter change ships
   with a diff of its output against a fixed corpus of real `.gx` files,
   reviewed by a human, before merge.

5. **Pick one canonical representation per domain and hold every builtin
   in that domain to it**, starting with dates (ISO string, per the
   "auditable" philosophy — a raw Unix timestamp is opaque in a log line
   or a `say` statement; an ISO string is self-describing). This is
   worth generalizing into a standing rule for any future builtin module:
   the module's own doc comment already states an "accepts either
   representation" policy for *input* — the same policy should apply
   symmetrically to output, and that symmetry should be a review
   checklist item for new builtins, not something discovered per-module
   after a user gets burned.

6. **Give GX a minimal, explicit script-arguments story** (`argv()`),
   since its own toolchain (`gx run file.gx <args>`) already implies one
   exists. This is small, additive, and removes a real gap for anyone
   using GX as a general-purpose CLI scripting language, which the
   report shows is a real usage pattern (per-contact agents, an
   interactive dashboard) that GX is currently fighting rather than
   supporting.

Everything above preserves — and in most cases *reinforces* — the four
pillars called out in the brief (simplicity, transparency, deterministic
behavior, auditable execution, minimal runtime magic): the fixes make
GX's own execution model behave the way its AI-facing features already
do, rather than adding new magic anywhere.
