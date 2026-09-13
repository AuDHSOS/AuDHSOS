# writing style

Keep everything in US English. Write clear and distinct sentences. Get to the point.
Do not write rule of thumb sentences. Especially not in headlines.

While you work report only bugs / critical findings / problems etc. Keep it short.

In your final summary message report your done work.
Keep it also short. Overall keep your messages short.
Your final summary message is not only short but shorter.
Overall keep your messages shorter.

Keep comments in source code also short or shorter.

If you encounter a long source code comment then make it shorter.

Really do a "reduce to the max" without losing information.

## reference register, not prose

This is what "reduce to the max" means. The failure is not length, it is
restatement: one fact written three times as the fact, its reason, and a
closing sentence.

- One checkable fact per sentence. A fact is something the reader can act
  on or verify. Everything else goes.
- Reasons are subordinate clauses, never their own sentences.
- No framing sentences, no closing sentences. A section opens on its
  content and stops when the content stops.
- Recommend; do not weigh. Name the option not taken in a clause.
- Resolve a question rather than documenting it, where reading the code
  settles it.
- Test before writing a sentence: would a reader who knows this project
  lose anything if it vanished? If not, it is rhythm.

The older documents under `docs/` argue in long prose. They are not the
model. `docs/15-the-disk-on-the-machine.md` is.

## sentence shape

- Name the actor and say what it can or cannot do. The subject is
  something in the system that acts — a driver, the root task, the
  kernel, a client. Never `nothing`, `there is`, `it`. Test: can the
  sentence become "may X do Y?" If not, find the actor the reader cares
  about. This is also the frame of a capability system, where every
  question is who may do what.
- Every sentence survives being quoted alone. Two rules follow from it:
  no stranded preposition or particle without its noun (`in`, `out`,
  `there`, `through`, `over`, `on it`); and no pronoun reaching into the
  sentence before — write the noun again.
- State what exists, not what an actor refrains from doing. `never` and
  `no longer` are habitual and imply a choice; code has the call or it
  does not.
- No metaphor as a verb with a subject that could act. A dead metaphor
  revives there: `the bus walk finds a device` is the name of an
  operation, `it walks the bus` stages a body. Use the domain's own word
  — `enumerate` for a bus, not `walk`.
- No spatial metaphor for access. A device is not a building and has no
  way in. Name the operation.
- Do not shorten by swapping a precise noun for a short verb of human
  action. Plain English for machines means dropping the metaphor, not
  the syllables.
- Write list items as label, colon, fact — not as sentences. Sentences in
  a list acquire a closing cadence and read as maxims.

## planning documents

`docs/15-the-disk-on-the-machine.md` is the house form. Follow it for
every new planning document and for every rewrite of an old one.

Structure, in this order:

1. `How to read this document`: one paragraph naming the fixed parts of a
   step.
2. `Terms`: a table. Every term used later, defined here.
3. `Goal`: what is true at the end that is not true now, numbered.
4. `What is already built`: a table per group, with where each piece was
   decided.
5. `What is missing`: a table of what is missing and why.
6. Decisions, numbered `D1`, `D2`. Each states the decision first, then
   the reasons numbered, then the option not taken and its cost.
7. `The order of the steps`: a table with status, dependency and size.
8. One section per step.
9. `Risks`: a numbered table of risk, effect, and what reduces it.

Every step section carries these parts, in this order: Status, Depends
on, Size, Needs, Does, Done when. A step that creates something nameable
carries `Produces` between Does and Done when.

Rules that produce this form:

- One name per thing, used everywhere. Never vary a word for style; each
  variation costs the reader a lookup.
- Procedures are numbered steps, not prose.
- A table wherever a sentence would carry two dimensions at once.
- State what the reader could infer. Inference is work.
- Label a reason as a reason. Do not fold it into the sentence that
  carries the fact.
- No metaphor, including dead ones: `falls out of`, `the line runs
  between`, `sits in`.
- A reference names a path and a line, not a bare number.
- `Done when` states an observable result, not an intention.

This form is longer than prose by about half again. Nothing is added; the
implicit becomes explicit.

Use `Big O notation` / classify algorithms by their run time. The user understands it.

# reference documents (RFCs etc.)

Cite standards always by lookup in the concrete document.

Check `docs/**` what is already there.

If the subdirectory is missing read `docs/rfc/README.md` to see how RFC documents are
handle and reflect the style.

IF a document is missing read `README.md` of the subdirectory and create
the file in the subdirectory (mandatory).

# build an check commands

before a `git commit` the check must exit with 0.
Do not take any shortcut so that exit 0 in the check is reached.

Every cargo call on macOS is run through wrappers in `tools/`.
`cargo` directly is not the nightly build.

- `sh tools/xtask.sh <subcommand>` e.g. `lint`, `test`, `doc`, `fuzz`, `--help` for help
- `sh tools/xtask-check.sh` — full check before every commit. Must exit with 0. Takes around 3 minutes when compiled

If `cargo fmt --all` fails use `cargo fmt -p <name>` to format one package (parallel sessions).

(Optional read for reason: `ER-1` in `extended-read.md`.

# Python

* `uv` is the only tool for managing packages or venvs
* do not create permanent python scripts
