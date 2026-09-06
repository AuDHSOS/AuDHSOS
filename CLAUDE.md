# Anrede
Sie reden mich mit sie an. Genauso werde ich sie mit sie anreden.

# Schreibstil

Schreiben sie immer klar und deutlich. Kommen sie immer auf den Punkt.
Keine gnomischen Merksätze.
Schreiben sie nicht von`Naht`. Sondern nutzen sie den englischen Begriff.
Es sei denn, es wird von der Stoff-Naht geschrieben.
Fachbegriffe, insbesondere aus der Informatik, bleiben englisch. In der deutschen Kommunikation werden Rust- und Systembegriffe nicht übersetzt: Crate, Trait, Slice, Borrow, Lifetime, Frame, Page, Handle, Endpoint, Guard Page, Fuzzing, Mangling, Kernel, Memory, Page Table, seed corpus, seed, seeds, corpus entry, corpus file. Sie werden als Fremdwörter dekliniert („des Crates", „die Traits").

# Referenzdokumente

Standards nie aus dem Gedächtnis zitieren, sondern erst nachsehen, ob das
Dokument geladen ist:

- RFCs liegen unter `docs/rfc`.
- Was OASIS herausgibt, liegt unter `docs/oasis`; dort liegt die
  virtio-Spezifikation.

Wenn das Dokument fehlt, lesen sie die `README.md` des jeweiligen
Verzeichnisses, laden es so wie dort beschrieben und lesen es
anschließend.

# Build- und Check-Kommandos

Der Check muss laufen, bevor ein `git commit` erstellt wird.

Alle Cargo-Aufrufe dieses Projekts laufen über die Wrapper in `tools/`, nie
über ein blankes `cargo`:

- `sh tools/xtask.sh <subcommand>` — z. B. `lint`, `test`, `doc`, `fuzz`
- `sh tools/xtask-check.sh` — der volle Check, muss vor jedem Commit grün sein
  (warm ca. drei Minuten)

Grund: `/opt/local/bin/rustc` (MacPorts) steht auf dieser Maschine vor
`~/.cargo/bin` im `PATH`, der Workspace verlangt aber die gepinnte Nightly.
Die Wrapper setzen den `PATH` gerade, schalten Pager und Farbe ab und
`exec`en dann Cargo: Ausgabe und Exit-Status sind die des xtask, `&&` und
`$?` gelten also. Ein voller Check schreibt warm rund dreitausend Zeilen;
`sh tools/xtask-check.sh --quiet` macht daraus eine Zeile pro Schritt und
zeigt die Ausgabe nur von dem Schritt, der fehlschlägt.

`cargo fmt --all` scheitert, sobald eine parallele Session ein halb
geschriebenes Crate im Workspace liegen hat. Einzelne Packages formatieren:
`cargo fmt -p <name>`.
