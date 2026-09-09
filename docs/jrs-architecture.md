<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Manuel Baesler and contributors -->

# jrs: Zielarchitektur und Migrationsplan

Stand: 9. September 2026. Status: Architekturvorschlag, noch keine Implementierungsfreigabe für sämtliche beschriebenen Erweiterungen.

Dieses Dokument beschreibt eine wartbare, erweiterbare, testbare und auf hohe Performance ausgerichtete Architektur für jrs. Es basiert auf einer lesenden Prüfung des vorhandenen Workspaces. Es verändert nicht dessen laufende Implementierung. Die Konformitäts- und Performance-Ziele bleiben bestehen; die frühere Beschränkung auf eine reine Automaton-Engine ist durch die ausdrückliche Freigabe von `regex-bt` ersetzt. Vorgeschlagene Crates, Typen und APIs sind ausdrücklich Zielzustand, sofern sie nicht als vorhanden bezeichnet werden.

Die Architekturentscheidung lautet: **ein semantisch einheitlicher ECMAScript-Core mit klarer Agent-/Realm-Zuordnung, einem gemeinsamen Objektmodell, expliziten Ausführungsfortsetzungen und getrennten Host-/Webplattform-Adaptern**. Optimierungen setzen auf diesen Verträgen auf und dürfen keine zweite, abweichende Sprachsemantik etablieren.

## 1. Ziele, Grenzen und offene Produktentscheidungen

### 1.1 Ziele

- ECMAScript anhand der lokalen [ECMA-262-Fassung](ecma/ecma262.html) implementieren.
- Die Tests im lokalen Test262-Checkout vollständig und korrekt ausführen und bestehen.
- Die gewünschte Webplattform über WPT verifizieren, nicht über nachgebildete Assertions.
- Keine externen Software-Abhängigkeiten im Build oder in der ausgelieferten Runtime; wiederverwendbare Komponenten unter `crates/`.
- `crates/regex-bt` als RegExp-Backend von jrs verwenden; wiederverwendbare Bausteine aus `crates/regex` weiter nutzen.
- Hohe Performance bei überprüfbaren Memory- und Ausführungsgrenzen.
- Safe Rust in den Logic-Crates; jede notwendige Low-Level-Ausnahme bleibt klein, isoliert und ausdrücklich genehmigt.

Das Wort „schnell“ und die Aussage „alle WPT-Tests“ benötigen überprüfbare Abnahmekriterien. Die Architektur stellt diese Kriterien zur Entscheidung, erklärt aber keine kleinere Auswahl zum ursprünglichen Gesamtziel.

### 1.2 Entscheidungen vor einer vollständigen Abnahme

| Entscheidung | Stand beziehungsweise fehlende Festlegung | Festlegung oder Vorschlag |
|---|---|---|
| Regex-Backend | Entschieden: jrs darf `regex-bt` einschließlich Backtracking nutzen. Die frühere Automaton-only-Vorgabe entfällt. | `regex-bt` ersetzt `regex` als direktes Matching-Backend. Gemeinsame Syntax-/Datenbausteine bleiben wiederverwendbar. Vollständige RegExp-Konformität und wirksame Ressourcenlimits müssen weiterhin implementiert und nachgewiesen werden. |
| Webplattform | WPT umfasst auch DOM, HTML, Worker, Netzwerk, Origin-Regeln, Layout und weitere Browser-Funktionen. Ein Sprach-Core allein ist kein WPT-Produkt. | Sprachengine, Shell-Host und Browser-Host als getrennte Liefergegenstände planen. Die vollständige Inventarliste bleibt sichtbar, auch wenn zunächst nur Teilbereiche implementiert sind. |
| Performance | Drei Interpreter-Microbenchmarks belegen keine Konkurrenzfähigkeit gegenüber einer Produktionsengine. | Ein versioniertes Workload- und Zielhardware-Profil mit absoluten Budgets und Referenzvergleichen festlegen; siehe Abschnitt 15. |
| Standardscope | Der lokale ECMA-Draft, Test262-Staging, Intl-Tests und unterschiedliche WPT-Features sind nicht automatisch deckungsgleich. | Spec-Version, Testrevisionen, Plattformen und Feature-Matrix gemeinsam pinnen. Konflikte einzeln begründen und entscheiden, niemals durch stilles Überspringen lösen. |
| JIT und `unsafe` | Ein nativer JIT benötigt Executable Memory und Architektur-/OS-Code. Das ist nicht durch das bestehende Safe-Rust-Core-Verbot abgedeckt. | Zunächst den Interpreter optimieren. Ein JIT ist eine eigene spätere ADR und keine stillschweigende Erweiterung der Berechtigungen. |

Die Nutzung von `crates/regex-bt` ist ausdrücklich freigegeben und Bestandteil der Zielarchitektur. Der Backend-Austausch wird bei der Umsetzung explizit vorgenommen, nicht als automatischer Fallback nach einem fehlgeschlagenen Automaton-Match. `regex-bt` verwendet bereits Bausteine aus `crates/regex`; diese Wiederverwendung bleibt erhalten. Die Änderung dieses Dokuments allein schaltet das Backend im laufenden Code noch nicht um.

„Keine externen Abhängigkeiten“ bedeutet hier: keine Registry-/Git-Crates, kein eingebetteter Fremdengine-Code, kein versteckter System-JavaScript-Interpreter und kein Download während des normalen Builds. Rust-Toolchain, Zielbetriebssystem und dessen ausdrücklich genutzte System-ABI bleiben die technische Basis. Spezifikationen, Unicode-Daten und Test-Suiten sind versionierte Daten mit Herkunft und Lizenz, keine Ersatzimplementierungen. Externe Referenzengines dürfen als zusätzliche Entwicklungs-Oracles eingesetzt werden, sind aber nicht Voraussetzung für den eigenständig ausführbaren Projektcheck.

## 2. Ausgangspunkt und zu erhaltende Arbeit

Die vorhandene Implementierung bietet bereits erheblichen Nutzen: einen `no_std + alloc`-Core ohne `unsafe`, einen Bytecode-Interpreter, einen generationengeprüften Tracing-Heap, persistenten Realm-Zustand, Promise-Jobs, Iterator- und Constructor-Pfade sowie unabhängige Regex-, JSON-, Math-, UTF-16-, Event- und Timer-Komponenten. Diese Arbeit wird migriert, nicht pauschal ersetzt.

Die folgenden strukturellen Risiken sind im aktuellen Code erkennbar:

| Aktueller Bereich | Risiko bei weiterem Ausbau | Ziel |
|---|---|---|
| [vm.rs](../crates/jrs/src/vm.rs) und Untermodule | `Execution` vereint Ausführung, Heap, Intrinsics, Realms, Jobs und Web-Host-Zustand. Zustandsänderungen haben breite Auswirkungen. | Explizite Besitzer und engere interne Zugriffsrechte. |
| [object.rs](../crates/jrs/src/object.rs) | Ein großes Object trägt zahlreiche optionale Internal Slots, darunter Webobjekte. Neue Typen vergrößern alle gewöhnlichen Objekte. | Kompakter gemeinsamer Header und typisierte Payloads. |
| [value.rs](../crates/jrs/src/value.rs) | Sofortige native Callable-Identitäten und heapgebundene Funktionen benötigen unterschiedliche Property-Pfade. | Jede JavaScript-sichtbare Funktion ist ein echtes Function-Objekt. |
| [bytecode.rs](../crates/jrs/src/bytecode.rs) | Bytecode-Definition, Compilation und Laufzeitkonstanten sind gekoppelt; isolierte Scripts und Realm-Scripts haben unterschiedliche globale Bindungsmodelle. | Heapfreier Code-Vertrag und ein einziges Script-Bindungsmodell. |
| Native Roots und synchrone Reentry | Manuelle Root-Stack-Längen und Rust-Aufrufketten erhöhen Audit-Aufwand und führen zu zusätzlichen Native-Depth-Grenzen. | Root-Scopes und fortsetzbare Builtin-Frames. |
| [Limits](../crates/jrs/src/lib.rs) | Anzahlen begrenzen logische Ressourcen, aber nicht die tatsächlich allokierten Bytes. Intrinsics und Anwendung konkurrieren teilweise unter denselben kleinen Testbudgets. | Getrennte, messbare Budgets einschließlich Infrastrukturkosten. |
| Event-/Timer-Code im Core | Neue Web-APIs können die ECMAScript-Ausführung und deren Memory-Layout verändern. | Optionale Host-Pakete über einen definierten Native-Object-Vertrag. |
| CLI-Conformance-Runner | Der Runner kann durch falsche Completion-Logik scheinbare Konformität erzeugen. | Runner als eigenständig getestetes, fehlschlagendes Prüfwerkzeug. |

Diese Aussagen sind keine Behauptung, dass jeder aktuelle Pfad fehlerhaft ist. Sie identifizieren Grenzen, die vor Proxy, mehreren Realms, Modulen, TypedArrays und einem Optimizer stabilisiert werden müssen.

## 3. Architekturregeln

1. **Eine Semantik:** Script, Module, `eval`, dynamische Function-Constructors, Host-Aufrufe und optimierte Ausführung benutzen dieselben abstrakten Operationen und Completion-Regeln.
2. **Ein Besitzer pro Ressource:** Heap, Source, Code, Handles, Tasks und externe Buffer haben jeweils einen expliziten Owner und ein explizites Budget.
3. **Kein verstecktes JavaScript:** Jede Operation, die Getter, Proxy-Traps, Constructor oder Callbacks ausführen kann, ist als solcher Effekt erkennbar und kann suspendieren.
4. **Keine Borrow über Reentry:** Über einen möglichen JavaScript-Aufruf hinweg bleiben nur IDs, Kopien und registrierte Roots bestehen, keine Referenzen auf veränderbare Heap-Inhalte.
5. **Keine Spezialsemantik für Tests:** Harness-Code benutzt dieselben Pfade wie Anwendungscode. Test-Hooks bleiben explizite, nicht standardmäßig verfügbare Capabilities.
6. **Begrenzung vor Wachstum:** Allocation, Queue-Eintrag, Code-Erzeugung und Buffer-Erweiterung reservieren ihr Budget vor der Zustandsänderung.
7. **Optimierungen haben Guards:** Jeder Fast Path besitzt einen vollständigen Guard, einen gemeinsamen Slow Path und Mutation-/GC-/Exception-Tests.
8. **Crate-Grenzen folgen Verantwortung:** Kein Crate pro Methode. Eng gekoppelte GC-, Objekt- und Ausführungslogik bleibt zunächst gemeinsam in der Engine.
9. **Fehler bleiben unterscheidbar:** Sprach-Exceptions, fehlende Features, Budget-Abbruch und interne Invariant-Verletzung sind keine austauschbaren Fehlerarten.
10. **Messbare Migration:** Jeder Umbau hat einen reproduzierbaren Ausgangsstand, ein begrenztes Änderungsziel und überprüfbare Exit-Kriterien.

## 4. Zielstruktur und Dependency-Richtung

Die folgenden zusätzlichen Crates sind eine geplante Aufteilung. Ihre Anlage erfolgt erst mit der zugehörigen Migration und den Layering-Regeln.

```text
crates/
  jrs/                         öffentliche Rust-Fassade und Kompatibilitäts-API
  jrs-source/                  SourceText, Spans, Diagnostik, Source-IDs
  jrs-frontend/                Lexer, Parser, Early Errors, Scope-Analyse
  jrs-bytecode/                Code-Format, Konstanten, Metadaten, Verifier
  jrs-compiler/                Lowering, Optimierungen, Bytecode-Erzeugung
  jrs-engine/                  Agent, Heap, Objektmodell, VM, ECMAScript-Builtins
  jrs-web/                     Web-IDL-Bindings, Events, Timer, spätere Web-APIs
  jrs-host-os/                 AuDHSOS-Embedding und Capability-Anbindung
  regex/                      gemeinsame Regex-Bausteine und eigenständige Thompson-Engine
  regex-bt/                   bounded Backtracking-Backend für jrs; nutzt regex-Bausteine
  utf16/ math/ json/ time/     vorhandene wiederverwendbare Bausteine
  timer-queue/ event-target/   vorhandene reine Host-Algorithmen
  unicode/                    geplante Unicode-Daten und reine Lookup-Algorithmen
  support/testing/            vorhandene Model-/Property-Test-Werkzeuge
  support/fuzz/               vorhandene Fuzz-Infrastruktur
  tools/
    jrs/                      CLI; Script-, Test262- und WPT-Runner als getrennte Module
    jrs-data/                 geplante offline ausführbare Datengeneratoren
```

| Consumer | Erlaubte direkte Abhängigkeiten im jrs-Bereich |
|---|---|
| `jrs-source` | Keine Engine; nur reine Hilfsalgorithmen, falls tatsächlich benötigt. |
| `jrs-frontend` | `jrs-source`, Unicode-/UTF-16-Daten. |
| `jrs-bytecode` | `jrs-source`; keine Heap-Values und kein Compiler. |
| `jrs-compiler` | Frontend, Source, Bytecode; keine Engine. |
| `jrs-engine` | Bytecode, Source, Compiler für dynamische Compilation, `regex-bt` und weitere reine Algorithmus-Crates. |
| `regex-bt` | Gemeinsame Syntax-, Klassen-, Options- und Ergebnistypen aus `regex`; keine Engine-/Realm-Abhängigkeit. |
| `jrs` | Engine sowie die öffentlichen Compile-/Source-Typen; keine Web- oder OS-Abhängigkeit. |
| `jrs-web` | `jrs` und reine Web-Algorithmen; weder CLI noch OS-Syscalls. |
| CLI / `jrs-host-os` | `jrs`, optional `jrs-web`, jeweiliger Plattformadapter. |
| Test-Werkzeuge | Öffentliche API und ausdrücklich testbare interne Schnittstellen; niemals umgekehrt. |

Der Compiler darf für `eval` von der Engine aufgerufen werden. Daraus entsteht kein Zyklus: Compiler und Bytecode kennen weder Agent noch Heap.

Webobjekte dürfen keine Felder in allen Core-Objekten erzwingen. Der Core kennt nur den generischen Native-Object-Vertrag aus Abschnitt 12. Neue Browser-Subsysteme werden unter `crates/` getrennt, sobald ihre Zuständigkeit und unabhängig prüfbare API klar sind; sie werden nicht als neue große Sammlung optionaler Felder in `jrs-engine` eingebaut.

## 5. Ownership: Engine, Agent, Realm und Ausführung

```text
Engine / immutable configuration
  └─ Agent A — genau eine aktive Ausführung
       ├─ Heap und Root-Registry
       ├─ Realm 1 — Global Object, Global Environment, eigene Intrinsics
       ├─ Realm 2 — Global Object, Global Environment, eigene Intrinsics
       ├─ Execution Context Stack / Continuations
       ├─ Jobs, Pending Host Requests, Budget-Konten
       └─ Native-State-Registry

  └─ Agent B — eigener Heap, eigener Ausführungszustand

AgentCluster — ausschließlich ausdrücklich teilbare Ressourcen und Atomics
```

- Ein Agent ist anfangs thread-confined und nicht frei `Send`/`Sync`. Mehrere Worker benutzen mehrere Agents, nicht einen gemeinsam gesperrten JS-Heap.
- Ein Agent kann mehrere Realms besitzen. Objekte aus Realms desselben Agents können Identität behalten und einander referenzieren. Jede Realm erhält eigene Intrinsic-Objekte.
- Funktions- und Job-Metadaten tragen ihre Realm-Zuordnung. Die Realm eines Aufrufers ersetzt nicht automatisch die Realm des Callees oder des erzeugten Fehlers.
- Origin- und Zugriffsregeln sind Host-Policy. Ein anderer Realm ist nicht automatisch eine Sicherheitsgrenze; ein anderer Agent-Heap ist keine Berechtigung zur direkten Objektübertragung.
- Zwischen Agents werden Daten über Structured Clone, Transfer oder explizite Shared-Backing-Stores übertragen. Gewöhnliche Heap-Handles dürfen nicht übertragen werden.
- Ein Realm wird nicht durch `drop(RealmHandle)` sofort freigegeben, solange Funktionen, Jobs oder andere erreichbare Objekte seinen Zustand benötigen. Realm-Lifetime ist Teil des Tracing-Modells.
- `Runtime::run` wird eine Komfortfunktion: frischen Agent/Realm erzeugen, normales Script ausführen, Resultat nach klarer Lifetime-Regel liefern. Kein eigener Compiler-Modus mit anderer Global-Semantik.

Die Aufteilung folgt den gelesenen ECMA-Abschnitten zu [Realms](ecma/ecma262.html#sec-code-realms), [Execution Contexts](ecma/ecma262.html#sec-execution-contexts), [Jobs](ecma/ecma262.html#sec-jobs) und [Agents](ecma/ecma262.html#sec-agents). Die konkrete Rust-Ownership ist eine Implementierungsentscheidung, keine wörtliche Umsetzung der Specification Records.

## 6. Source, Frontend und Compiler

### 6.1 Verlustfreie Source-Verarbeitung

`SourceText` unterstützt UTF-8-Dateieingaben und UTF-16-Source aus JavaScript. Lone Surrogates dürfen nicht still durch Ersatzzeichen verändert werden. Eine Source-ID bezeichnet unveränderlichen Text. Semantische Source-Spans verwenden UTF-16-Codeunit-Offsets; `Utf8ByteOffset` und `Utf16Offset` sind verschiedene Typen. UTF-8-Eingaben behalten eine geprüfte Abbildung für Dateidiagnostik und Function-Source. Positionen aus beiden Darstellungen werden nie als austauschbare `usize`-Werte weitergereicht.

Source-Lifetime und gespeicherte Function-Source werden gemeinsam abgerechnet. Ein kleiner Closure darf nicht unbemerkt eine unbegrenzt große Quelltextmenge halten. Source-Caches sind budgetiert und frei gebbar; `Function.prototype.toString` behält trotzdem seinen vorgeschriebenen Inhalt.

### 6.2 Explizite Parse-Ziele

Das Frontend erhält einen `ParseGoal` statt versteckter Booleans: Script, Module, FunctionBody, AsyncFunctionBody sowie die später benötigten Generator-Varianten. Strictness, Annex-B-Verhalten, erlaubte Syntax und Scope-Kontext sind explizit.

Die Pipeline lautet:

```text
SourceText → Lexical Goals / Tokens → AST + Spans
           → Early Errors + Scope Graph → Lowering IR
           → Bytecode + Exception-/Source-/Liveness-Metadaten → Verifier
```

Lexer und Parser koordinieren die lexical goals für `/`, Templates und ähnliche kontextabhängige Formen. Destructuring, Cover Grammar und Early Errors werden nicht durch nachträgliche String-Heuristiken erkannt. Die Scope-Analyse unterscheidet lexikalische Bindungen, `var`, Parameter, Catch-Bindungen, Imports und dynamische Scopes.

Gültige, aber noch nicht implementierte Syntax bekommt eine eigene Diagnose. Sie darf nicht als nachgewiesener `SyntaxError` einen Parse-Negativtest bestehen. Parser-/Compiler-Rekursion bleibt entweder streng begrenzt oder wird für besonders tiefe Formen durch explizite Stacks ersetzt.

### 6.3 Code-Format und Wiederverwendung

`CodeBlock` enthält keine Realm-spezifischen Objekte, `Value`-Handles oder nativen Function-Identitäten. Konstanten sind beispielsweise Number-Bits, String-Codeunits, Namen, Regex-Source/Flags und Referenzen auf weitere CodeBlocks. Die Instanziierung erzeugt Realm-/Agent-spezifische Daten separat.

Bytecode-Verifikation prüft Sprungziele, Register-/Stack-Grenzen, Exception-Regionen, Konstantentypen, erlaubte Operandenkombinationen und konsistente Root-Metadaten. Intern erzeugter Code wird in Tests ebenfalls verifiziert. Ein persistenter Code-Cache benötigt Formatversion, Spec-/Feature-Profil, Quellhash und sichere Neuvalidierung.

Compile-Limits werden von Execution-Limits getrennt. Kompatible CodeBlocks können in mehreren Realms mit unterschiedlichen Laufzeitbudgets ausgeführt werden; Code-Anforderungen wie maximale Frame-Größe werden beim Einstieg geprüft. Die derzeitige Gleichheit aller Compile-/Runtime-Limits ist kein dauerhafter Cache-Vertrag.

## 7. Ein einheitliches Objektmodell

### 7.1 Values und Payloads

Alle JavaScript-Objekte, einschließlich Builtin-, Bound- und Host-Functions, erhalten echte Heap-Identität. Ein `BuiltinId` bezeichnet eine Implementierung, nicht das sichtbare Function-Objekt. Das beendet Constructor-Schattenobjekte und eigene Property-Sonderfälle pro Builtin.

Interne Values sollen klein und günstig kopierbar sein: Tags für Primitive sowie IDs für Strings, Symbols, BigInts und Objects. Ein 16-Byte-Layout ist ein zu messender Kandidat, keine Voraussetzung. NaN-Boxing oder Pointer-Kompression werden erst nach Portabilitäts-, Sicherheits- und Performance-Nachweis erwogen. Externe Values tragen weiterhin eine überprüfbare Agent-Zugehörigkeit.

Jedes Object hat einen kompakten gemeinsamen Header und eine eindeutige Payload-Art, etwa Ordinary, Array, Function, Proxy, TypedArray oder NativeObject. Function-interne Daten unterscheiden Script, Builtin, Bound und Host. Zusätzliche Informationen für seltene Fälle liegen in typisierten Erweiterungen, nicht in allen Instanzen.

### 7.2 Property-Engine

Ein zentraler Dispatcher implementiert die gelesenen [Object Internal Methods](ecma/ecma262.html#sec-ordinary-object-internal-methods-and-internal-slots): `Get`, `Set`, `HasProperty`, `GetOwnProperty`, `DefineOwnProperty`, `Delete`, `OwnPropertyKeys` und Prototype-/Extensibility-Operationen. Aufrufbarer Status und Constructibility gehören ebenfalls in diesen Vertrag.

Exotische Objekte implementieren klar definierte Abweichungen. Insbesondere:

- Array-Length- und Indexregeln unterscheiden Array-Indizes bis `2^32−2` von generischen arrayähnlichen Indizes bis zur `ToLength`-Grenze.
- String-Wrapper stellen ihre virtuellen Indizes korrekt bereit.
- Arguments-Mapping wird über eigene Payload-Daten verwaltet.
- TypedArray-Views validieren Detachment, Bounds und Buffer-Länge am vorgeschriebenen Punkt.
- Proxy-Traps laufen über dieselben fortsetzbaren Operationen und prüfen anschließend die Target-Invarianten. Nach einer Trap müssen relevante Descriptors gegebenenfalls neu gelesen werden.

Builtin-Algorithmen dürfen nicht direkt in Object-Maps schreiben, um Getter, Setter, Proxy oder Descriptors zu umgehen. Der direkte Storage-Zugriff ist nur innerhalb der Property-Engine beziehungsweise eines beweisbar passenden Fast Paths zulässig.

### 7.3 Shapes und Elements

Der zunächst vorhandene generische Property-Speicher bleibt der verlässliche Slow Path. Danach werden immutable Shapes und Slot-Arrays für gewöhnliche Objekte eingeführt. Eine Shape beschreibt Layout, Attribute, Property-Reihenfolge und Prototype-Abhängigkeit, nicht die veränderbaren Property-Werte. Häufige strukturelle Änderungen führen in Dictionary Mode; diese Objekte müssen nicht ständig neue Shapes erzeugen.

Arrays erhalten getrennte Elements-Speicher mit expliziten Zuständen: packed, holey und sparse/dictionary. Hole und `undefined` sind verschieden. Ein Fast Path darf nur bei passender Shape, korrektem Prototype-Zustand und fehlenden relevanten Accessors benutzt werden. Prototype-Mutationen invalidieren abhängige Caches.

### 7.4 Intrinsics

Eine zentrale `IntrinsicId`-Registry erzeugt pro Realm Function-Objekte, Prototype-Links, Descriptors und Alias-Beziehungen. Initialisierung erfolgt in zwei Schritten: benötigte Objekte anlegen, danach Referenzen und Properties verbinden. So werden Zyklen ohne halb veröffentlichte Lazy-Objekte aufgebaut.

Lazy-Initialisierung ist nur als geprüfte Optimierung erlaubt. Reihenfolge von Property-Enumeration, Mutationen, Identität und Realm-Zuordnung müssen der definierten Initialisierung entsprechen. Alias-Identitäten wie `Array.prototype.values` und `Symbol.iterator` werden als Datenbeziehung erfasst und systematisch getestet.

### 7.5 Interne Modulgrenzen der Engine

| Modul | Besitzt | Darf nicht |
|---|---|---|
| `agent` / `realm` | Lifecycle, Realm-Registry, Intrinsic-Wurzeln, laufende Ausführung. | Algorithmusdetails jedes Builtins aufnehmen. |
| `heap` / `roots` | Storage, GcIds, Tracing, Root-Scopes, Barrier-Mechanik. | Getter, Proxy-Traps oder Host-Callbacks ausführen. |
| `objects` / `environments` | Property-Storage, Internal Methods, Bindungs- und Scope-Semantik. | Policy oder OS-Zugriffe entscheiden. |
| `execution` | Frames, Dispatch, Completion, Suspension und Unwinding. | Separate Fast-Path-Versionen der Sprachregeln pflegen. |
| `builtins` | Spezifikationsalgorithmen und deren typisierte Continuations. | Heap-Maps oder fremde Frames unmittelbar verändern. |
| `modules` | Module-Records, Linking und Evaluation. | Dateien oder URLs ohne Host-Request laden. |
| `embedding` | Validierte Handles, Native-Type-Registry und Host-Protokoll. | Nicht registrierte Rust-Referenzen im Heap verstecken. |
| `feedback` | IC-Zustand, Profilzähler, Code-Spezialisierungen. | Ohne Guard Zugriffsergebnisse oder Lookup-Regeln verändern. |

Diese Module gehören zunächst in dasselbe Engine-Crate. Ihre API-Sichtbarkeit ist eng; ein allgegenwärtiges `pub(crate)` auf allen Feldern ersetzt keine Grenze. Weitere Crate-Aufteilungen werden erst vorgenommen, wenn ein stabiler, zyklusfreier Vertrag und unabhängige Tests existieren.

## 8. Ausführung, Completions und Builtin-Continuations

Der Interpreter bleibt zunächst die portable Referenzausführung. Das bestehende Stack-Bytecode ist der Migrationsstart. Ein kompakter registerbasierter Lowering-Pfad kann später Operandenkopien und Dispatch reduzieren; er wird nur nach einem A/B-Nachweis Standard. Dauerhaft gepflegt wird ein produktives Format, nicht eine beliebig wachsende Sammlung gleichberechtigter Interpreter.

Ausführung besteht aus expliziten Frames:

- Script-/Function-Frame mit Code, Environment, PC und Realm.
- Builtin-Frame mit Algorithmusphase und gehaltenen Werten.
- Host-Continuation mit registriertem Zustand und überprüfbarem Resume-Token.
- Suspendierte Generator-/Async-Frames mit vollständigem Exception-/Root-Zustand.

Ein Builtin, das JavaScript ausführen kann, gibt beispielsweise `Call`, `Construct`, `Get`, `Await` oder `Return` als nächsten Schritt an den zentralen Dispatcher zurück. Der Dispatcher setzt nach der Antwort den Builtin-Frame fort. Die phasenbezogenen Zustände bilden einen typisierten Enum; ungültige Kombinationen werden nicht durch lose Integer und Flags codiert.

Beispiel für die Zustandsfolge von `Array.from`:

```text
Mapper prüfen → Iterator-Methode lesen → Ergebnis konstruieren
              → Iterator erwerben → Step → Map → Define → nächster Step
                                              └─ Sprachfehler → IteratorClose → ursprüngliche Completion
```

Dadurch wächst bei langen Callback-Ketten der begrenzte VM-Stack statt der Rust-Stack. Gleiches gilt für Property-Getters, Proxy-Traps und dynamische Constructor-Ketten. Interne JavaScript-Helper dürfen als Zwischenlösung bleiben, laufen aber durch denselben Compiler und dürfen weder falsche Realms noch beobachtbare künstliche Frames erzeugen.

Completion-Modell:

- ECMAScript-Completions: Normal, Throw, Return, Break und Continue mit exakt erhaltenen Werten/Zielen.
- Engine-Steuerung: auszuführen, fortgesetzt, suspendiert oder beendet.
- Terminale Engine-Fehler: Budget-Abbruch, fehlendes Feature im Diagnosebetrieb, ungültiger Host-Vertrag und interne Invariant-Verletzung.

Nur Sprach-Completions durchlaufen die vorgeschriebenen Catch-/Finally-/IteratorClose-Regeln. Ein terminaler Abbruch startet keine beliebige zusätzliche Cleanup-JavaScript-Ausführung. Deterministisches internes Resource-Cleanup erfolgt ohne User-Code; Wiederverwendbarkeit beziehungsweise Poisoning des Agents wird ausdrücklich festgelegt und getestet. Rust-Panics sind keine JavaScript-Exceptions.

## 9. Heap, Roots und Memory-Budgets

### 9.1 Zunächst präzises, nicht verschiebendes Tracing

Der vorhandene generationengeprüfte Mark/Sweep-Ansatz bleibt Ausgangspunkt. Stabile `GcId`s trennen logische Identität von Storage-Position. Generationen dürfen nach Überlauf nicht wiederverwendet werden. Der Agent besitzt Heap, Allocation-Verwaltung und Root-Registry.

Zu den Root-Klassen gehören Frames, suspendierte Continuations, Realm-Intrinsics, Environment Records, Jobs, Tasks, Moduldaten, aktive Host-Requests und explizit persistente Host-Handles. Schwache Referenzen sind keine gewöhnlichen Roots. WeakMap-Verarbeitung benutzt einen Ephemeron-Fixpoint; WeakRef/FinalizationRegistry benötigen zusätzlich die jeweiligen Job-/Checkpoint-Regeln.

Native Root-Scopes registrieren ihre Values in einer geprüften Stack-Struktur. Das Pattern „Länge merken, pushen, später irgendwo truncate“ wird in eine kleine interne API gekapselt. Suspendierender Zustand gehört dagegen in einen tracbaren Frame und darf nicht von einem Rust-Stack-Guard abhängen.

### 9.2 Safe-Point-Vertrag

Operationen werden mindestens nach diesen Effekten unterschieden:

| Effekt | Zulässige Implementierung |
|---|---|
| `NoGc / NoJs` | Kurze Heap-Borrows und reine Arithmetic sind erlaubt. |
| `MayAllocate` | Allocation kann an einem dokumentierten Safe Point GC auslösen; Eingaben müssen rooted sein. |
| `MayCallJs` | Keine lebenden Heap-Borrows; Fortsetzung und Zwischenwerte müssen tracbar sein. |
| `MaySuspend` | Vollständiger Zustand liegt in Agent-eigenen Frames; keine Referenzen auf den Rust-Aufrufstack. |

Diese Klassifikation ist Teil der APIs, Reviews und Tests, nicht bloß ein Kommentar auf einzelnen Methoden. GC-Stresstests sammeln an jedem zulässigen Safe Point. Ein No-GC-Bereich darf kein beliebig großes Builtin oder eine gesamte I/O-Operation umfassen.

### 9.3 Evolution des Collectors

Store-APIs erhalten von Anfang an eine zentrale Write-Barrier-Schnittstelle. Im initialen Stop-the-world-Collector kann sie trivial sein. Später erlauben dieselben Stellen einen generational/incremental Collector mit remembered sets und überprüften Barrier-Invarianten.

Compaction und Moving GC werden erst nach Handle-, Root- und Host-API-Stabilisierung eingeführt. Parallel-/Concurrent-GC ist kein Frühziel: Es würde Write Barriers, Thread-Sicherheit und Debugging erheblich erweitern, bevor die Sprachsemantik stabil ist.

### 9.4 Budgets nach Besitzer und Bytes

Budgets werden getrennt für Source, AST/IR, Code, Heap-Storage, temporäre Builtin-Listen, Host-Handles, externe Buffer, Jobs, Tasks, Module und Regex-Compilation geführt. Neben Anzahlen werden reservierte Kapazitäten in Bytes erfasst. Cache-Einträge und Scratch-Buffer zählen mit.

Ein Budget-Ticket wird vor Allocation reserviert, bei Fehlschlag zurückgegeben und beim Owner-Wechsel übertragen. `try_reserve` allein genügt nicht: Auch nicht fallibel allokierende Container und `Rc`-/Box-Erzeugung benötigen eine kontrollierte Strategie. Harte Memory-Isolation erfordert vollständig kontrollierte Allocation-Pfade und gegebenenfalls einen OS-Prozess mit Address-Space-Quota. Bis dahin ist OOM-Recoverability nicht bewiesen.

Intrinsic-Bootstrap und User-Allokationen werden separat ausgewiesen, zählen aber gemeinsam gegen die harte Gesamtgrenze. Ein wachsender Builtin-Katalog darf nicht zu fortgesetztem blindem Erhöhen der Testbudgets führen. GC-Tests reservieren definierte Headroom-Budgets oder erzwingen Safe Points direkt.

## 10. Strings, Numbers, BigInt, Unicode und Buffer

- Strings bewahren UTF-16-Codeunits einschließlich Lone Surrogates. Ein One-byte-Speicher für Latin-1 und ein Two-byte-Speicher sind sinnvolle erste Repräsentationen; Slice-/Rope-Formen erst mit Depth-, Retention- und Flattening-Budgets.
- Property-Namen werden als Atoms verwaltet. Große oder einmalige Nutzstrings werden nicht automatisch permanent interned. Atom-Lifetime und Quota bleiben messbar.
- Die vorhandenen Math-/UTF-16-Algorithmen bleiben unabhängig testbar. Number-Konvertierung, Rundung, signed zero und NaN-Verhalten erhalten einen einzigen gemeinsamen Vertrag.
- JavaScript-BigInt ist ein eigener numerischer Typ. Ein vorhandener Crypto-Bignum-Kern ist nur nach Prüfung von Semantik, API, Lizenz und Timing-Anforderungen wiederverwendbar; Crypto-Konstantzeit ist nicht automatisch das passende allgemeine BigInt-Design.
- Unicode-Daten werden offline aus gepinnten Datenquellen erzeugt und als Tabellen ausgeliefert. Generator, Quelle, Lizenz, Hash und Unicode-Version sind Teil des Artefakts. Parser, String-Operationen und Regex benutzen dieselbe Version.
- Date trennt reine Kalender-/Zeitrechnung, Systemzeit und Zeitzonen-Policy. Die in der vollständigen Test262-Auswahl enthaltenen Intl-Bereiche benötigen zusätzlich eine ECMA-402-Implementierung mit versionierten Locale-, Kalender- und Zeitzonendaten; ein Unicode-Case-Mapping-Crate allein ersetzt sie nicht.
- `ArrayBuffer` besitzt einen expliziten Backing Store. Views halten Handle, Offset, Elementtyp und gegebenenfalls length-tracking Status. Detach/Resize erhöhen eine Version; Caches und aktive Operationen müssen nach JS-Reentry die vorgeschriebenen Bounds erneut validieren.
- SharedArrayBuffer und Atomics gehören zum AgentCluster-Modell. Sie dürfen nicht durch Freigabe gewöhnlicher Objekt- oder Slice-Referenzen zwischen Threads nachgebildet werden.

## 11. Regex als isolierte Sicherheitskomponente

`crates/regex-bt` ist das vorgesehene Matching-Backend von jrs. Es erhält Pattern/Text, Optionen und explizite Budgets; es kennt weder Realm noch `Value`, Properties oder Callbacks. Die iterative Backtracking-VM verwendet explizite Choice-/Assertion-Stacks statt rekursiver Matcher-Aufrufe auf dem Rust-Stack. Die Komponente bleibt unabhängig testbar und auditierbar.

`crates/regex` bleibt als Quelle gemeinsamer Syntax-, Zeichenklassen-, Assertion-, Options-, Fehler- und Ergebnistypen erhalten. Seine eigenständige Thompson-Engine kann weiterhin von anderen Consumers und als Differential-Oracle für den regulären gemeinsamen Umfang verwendet werden. jrs benötigt keinen zweiten Parser und keine zweite RegExp-Objektsemantik. Eine spätere Automaton-Beschleunigung innerhalb dieses Backend-Vertrags muss die Backtracking-Ergebnisse einschließlich Capture-Priorität erhalten und eigene Guards sowie Tests besitzen.

Der ECMAScript-Adapter besitzt dagegen RegExp-Intrinsics, sichtbare Getter, `lastIndex`, Species, Symbol-Dispatch, Custom-`exec`, Capture-Objekte und Replacement-Callbacks. Native Matcher-Fast-Paths werden nur nach vollständigen Property-/Prototype-Guards verwendet und nach Reentry erneut geprüft.

Backtracking bietet keine allgemeine lineare Laufzeitgarantie; adversarielle Patterns können exponentielle Arbeit verlangen. Deshalb gelten verbindliche Grenzen für Pattern-/Input-Größe, Compilation, ausgeführte Matcher-Schritte, Choice-/Assertion-Frames, Capture-/Register-Snapshots und allokierte Bytes. Das Work-Budget gilt gemeinsam für sämtliche Suchstarts, Vergleiche, Backtracking-Schritte und Capture-Kopien und wird gegen das Agent-Budget abgerechnet. Wiederholte Suchen innerhalb eines Builtins dürfen dieses Budget nicht zurücksetzen.

Limit-Erschöpfung ist ein expliziter Resource-Abbruch, niemals „kein Match“. Ein Engine-Wechsel oder Retry darf verbrauchte Budgets nicht verwerfen. Compilation, Unicode-Daten, Ergebnisaufbau und JavaScript-Callbacks benötigen zusätzlich ihre eigenen Budgets. Die Grenzen verhindern unbegrenzte Arbeit innerhalb des geprüften Budget-Vertrags, sind aber keine Behauptung vollständiger ReDoS-Abwehr oder harter OOM-Isolation; dafür gelten auch die Allocation- und Host-Grenzen aus Abschnitt 9 und 12.

Die Backend-Entscheidung aus Abschnitt 1.2 ist getroffen. Sie beseitigt den Architekturkonflikt bei nichtregulären Features, ersetzt jedoch nicht deren Implementierung: Der vorhandene `regex-bt`-Stand ist noch keine vollständige ECMAScript-RegExp-Engine. Fehlende Unicode-/Case-Folding-/Named-Group- und Legacy-Semantik wird weiterhin transparent erfasst. Vor Abnahme sind unabhängiges Fuzzing, Worst-Case-Budgettests, Capture-/Lookaround-Tests und die vollständigen einschlägigen Test262-Fälle erforderlich.

## 12. Host-API und Native Objects

### 12.1 Explizite Capabilities

Der Host stellt nur ausdrücklich eingerichtete Capabilities bereit: Output, monotone Zeit, Wall Clock, Entropy, Module-Resolution/Loading, Compilation-Policy, I/O und Exception-/Rejection-Berichte. Unterschiedliche Uhrbegriffe bleiben getrennt. Ein Test-Host kann virtuelle Zeit bereitstellen; ein WPT-Abnahmelauf darf keine bestandenen Echtzeit-Assertions durch erfundene Zeit vortäuschen.

Der Core implementiert keinen Dateizugriff, DNS, Socket und keine blockierenden Sleeps. Fehlende Capabilities werden nicht still durch globale OS-Funktionen ersetzt. Policy-Denial und I/O-Fehler haben API-spezifische Sprachsemantik; ein invalider Host-Handle ist dagegen eine Verletzung des Embedding-Vertrags.

### 12.2 Handle- und Borrow-Vertrag

Das öffentliche API-Ziel besteht aus `AgentHandle`, `RealmId`, lokal gerooteten Values und expliziten Persistent Handles. Interne Values sind keine frei übertragbaren Rust-Besitzobjekte.

- Handles werden auf Agent, Generation und Typ geprüft.
- Ein lokaler Root-Scope hält Werte bis zum Ende des Scopes; Speicherung darüber hinaus verlangt einen Persistent Handle.
- Persistente Handles haben eine explizite Freigabe und eigene Quota; jeder gelesene Property-Wert wird nicht automatisch dauerhaft retained.
- Rust-Borrows auf Heap- oder Buffer-Inhalte gelten nur in einem kurzen No-JS-/No-GC-Zugriff.
- Promise-/Host-Completion-Nachrichten tragen Tokens und Daten, keine ausgeliehenen Rust-Referenzen.
- Cancellation, verspätete Antworten, doppelte Completion und Shutdown werden durch Generationen beziehungsweise Request-Zustände erkannt.

Synchronous Host-Reentry ist nicht grundsätzlich verboten: Web-IDL-Konvertierungen und DOM-Algorithmen können es benötigen. Erlaubt ist ausschließlich der zentrale Continuation-Dispatcher ohne gleichzeitig lebenden Rust-Borrow auf den Agent. Eine ausstehende Host-Anforderung beendet dagegen nicht automatisch den aktuellen JS-Job; interne Suspension und sichtbares `await` sind getrennte Vorgänge.

### 12.3 Tracing von Native Objects

Ein Native Object referenziert `NativeTypeId` und `HostStateId`. Ein pro Agent registrierter Native-State-Store verwaltet dessen Zustand. Der zugehörige Tracer darf nur starke/schwache Referenzen beschreiben, keinen JS-Code ausführen und nicht beliebig allokieren.

Wenn ein Wrapper markiert wird, werden seine Native-Edges besucht. Unabhängige Host-Roots müssen explizit registriert werden. Native-Zustand darf nicht sämtliche Wrapper permanent rooten: JS↔DOM-Zyklen sollen durch gemeinsames Tracing sammelbar sein. Cleanup nach Sweep ist eingeschränkt und darf keine willkürliche JavaScript-Ausführung starten.

Mit diesem Vertrag kann `jrs-web` Native Objects bereitstellen, ohne dass die Engine DOM-Typen importiert oder ein Crate-Zyklus entsteht.

### 12.4 Öffentliche API-Verträge

Die Namen sind ein API-Entwurf, keine bereits verfügbare Rust-Signatur:

| Operation | Eingabe / Ergebnis | Verpflichtung |
|---|---|---|
| `compile` | Source, ParseGoal, CompileBudget → verifizierter CodeBlock oder Compile-Diagnose. | Keine Ausführung, keine I/O-Seiteneffekte und keine Realm-Values. |
| `create_realm` | Agent, Realm-Konfiguration → RealmId. | Neue Intrinsic-Identitäten und explizite globale Bindungen. |
| `evaluate` / `call` | RealmId, Code beziehungsweise gerootete Values → Value, Throw, Yield/Pending oder Termination. | Realm-Auswahl und Host-Turn-Modus sind explizit; keine versteckte neue Realm. |
| `resume` | Agent, gültiger Continuation-Token, typisierte Antwort → nächste Ausführungsentscheidung. | Exactly-once, Owner-/Generation-Prüfung, keine fremden Heap-Handles. |
| `checkpoint` | Agent und zulässiger Host-Checkpoint. | Reentrancy-Regel und Job-Realm erhalten; nicht implizit nach jedem Rust-Property-Read. |
| `poll` / `run_one_task` | Host-Scheduler-Zustand → fällige Arbeit oder nächste Deadline. | Keine I/O-Warteschleife im Core; nur zulässige Task-Ausführung. |
| `root` / `persist` / `release` | Lokales Value beziehungsweise Handle. | Lifetime und Memory-Kosten sichtbar; Freigabe und doppelte Freigabe eindeutig definiert. |
| `shutdown` | Agent und Shutdown-Policy. | Neue Arbeit ablehnen, Pending-Requests invalidieren, Ressourcen freigeben; Late-Completions sicher verwerfen. |

Eine Convenience-API darf mehrere Operationen kombinieren, muss ihren Checkpoint- und Lifetime-Vertrag aber benennen. Ein `eval`-Aufruf, der nur diagnostischen Text zurückliefert, darf nicht wie eine API aussehen, die langlebige Object-Handles liefert.

Safe Rust und generationengeprüfte Handles begrenzen Memory-Safety-Risiken; sie sind keine vollständige Sandbox gegen CPU-Verbrauch, Host-Fehler, Side Channels oder Angriffe auf das gesamte Betriebssystem. Nicht vertrauenswürdige Skripte erhalten für harte Isolation einen eigenen OS-Prozess beziehungsweise eine geeignete Capability-Domain. Host-Calls müssen selbst begrenzt sein: VM-Fuel kann eine blockierende oder fehlerhafte Rust-Callback-Implementierung nicht unterbrechen.

## 13. Jobs, Tasks und Webplattform

ECMAScript-Jobs und Host-Tasks werden nicht in eine einzige FIFO-Queue zusammengeworfen. Der Agent liefert die Mechanik; der Host definiert zulässige Checkpoints und Task-Auswahl anhand seiner Spezifikation.

Erforderliche Regeln:

- Run-to-completion pro JS-Job; keine Timer-Tasks mitten in einer synchronen Berechnung.
- Reentrancy-geschützter Microtask-Checkpoint und spezifikationsgerechte Rejection-Meldung.
- Ausführungs-Realm, Callback-Realm und Host-Settings werden getrennt transportiert.
- Timer-Nesting gehört zum gerade ausgeführten Timer-Task. Microtasks erben diesen Zustand nicht.
- Gleiche Timer-Deadlines behalten die vorgeschriebene Reihenfolge; Cancellation entfernt beziehungsweise invalidiert die richtige Registrierung.
- Wiederholte Timer, Callback-Exceptions und anschließende Checkpoints haben eigene Zustandsübergänge.
- GC-Liveness von AbortSignal, Observern und Timern wird aus der jeweiligen API abgeleitet, nicht aus pauschalem Retain-all.
- Eine Scheduling-Slice kann den Interpreter intern unterbrechen. Der Host darf dadurch im selben Agent keine sonst unzulässige Task-Interleaving-Semantik erzeugen.

`jrs-web` startet mit den vorhandenen Events/Abort/Timern. Weitere Pakete folgen in echter Dependency-Reihenfolge: Web-IDL-Konvertierungen und Brand-Checks; URL/Encoding; Buffer/Structured Clone; Fetch/Streams und Host-I/O; DOM-Baum und Event-Retargeting; HTML-/Window-/Worker-Lifecycle. Rendering, CSS, Layout, Accessibility und Grafik benötigen eigene Architektur- und Abnahmepläne.

Ein minimales `window`-Alias oder eine Sammlung von Methoden-Stubs ist keine Browser-Implementierung. Die Shell bleibt als Shell gekennzeichnet. Vollständiges WPT setzt einen realen Produktadapter einschließlich benötigter Server-, Origin-, Worker-, Testdriver- und gegebenenfalls Reftest-Infrastruktur voraus.

## 14. Module, dynamische Compilation und Debuggability

Der Module-Loader besteht aus getrennten Phasen: Specifier-Auflösung, Abruf, Parsing, Linking/Instantiation und Evaluation. Ein Registry-Key umfasst die Host-definierte Identität und relevante Import-Attribute, nicht nur einen Dateipfad. Module haben Live Bindings und einen expliziten Zustandsautomaten für Zyklen, Fehler und Top-Level Await.

Core-Code lädt keine URLs. Der Host gibt Source oder einen Pending-Token zurück. Module-Records und Promise-Abhängigkeiten bleiben während Suspension tracbar. Doppelte Ladung, rekursive Imports und fehlgeschlagene Evaluation dürfen keine halb initialisierten Bindungen veröffentlichen.

Direktes `eval`, indirektes `eval`, Function-Constructors und `$262.evalScript` sind unterschiedliche Operationen. Sie teilen Lexer/Parser/Compiler, aber nicht unzulässig denselben Scope-Kontext. Eine Compilation-Policy wird vor der vom Host zu verbietenden dynamischen Compilation am vorgeschriebenen Punkt geprüft.

Source-Spans, Bytecode-Disassembly, Stacktraces, Realm-/Job-IDs, GC-Ereignisse und Budget-Verbrauch gehören zur Diagnose. Log-Ausgabe darf keine implizite `ToString`-Konvertierung mit User-Code auslösen. Secrets aus Host-Capabilities werden nicht standardmäßig geloggt.

## 15. Performance-Architektur und Nachweispflicht

### 15.1 Optimierungsreihenfolge

1. Messbarer kompakter Value-/Object-/Frame-Zustand; Allokationen und Kopien reduzieren.
2. Atomisierte Property-Keys, Shapes und passende Array-Elements-Speicher.
3. Monomorphe und begrenzt polymorphe Inline Caches für Property-Zugriffe und Calls; begrenzter megamorpher Fallback.
4. Kompakter Bytecode, Spezialisierung häufiger Operationen und gemessene Superinstructions.
5. Generational/incremental GC, wenn Profile Allocation/GC als relevanten Engpass zeigen.
6. Optionaler Baseline-JIT; erst danach eventuell spezialisierende IR und optimierender JIT.

Inline Caches liegen pro Agent/Code-Instanz, nicht als veränderbare Heap-Verweise in global geteilten CodeBlocks. Sie prüfen Shapes, Prototype-/Buffer-Versionen, Realm-/Intrinsic-Abhängigkeiten und Descriptor-Art. Miss und Invalidation führen zur gemeinsamen Property-Semantik. Keine spekulative Annahme über unveränderbare Builtins.

Ein JIT benötigt vor Freigabe: W^X-Executable-Memory-Policy, präzise Safe-Point-/Root-Maps, Exception-Metadaten, Guards, Deoptimization in den Interpreter, Budget-/Interrupt-Checks und eigenständiges Differential-Fuzzing. Ohne diese Verträge wird kein nativer Code produktiv aktiviert.

### 15.2 Reproduzierbare Messungen

Der Benchmark-Katalog trennt Startup, Parse/Compile, Cold Run, Warm Run, Property-/Array-Last, Calls/Closures, Strings/Regex, Promises/Tasks, Allocation/GC, Peak Memory und reale Script-Workloads. Korrekte Ergebnisse werden immer geprüft. Ein Sum-Loop allein ist kein repräsentativer Katalog.

Vor jeder Performance-Änderung werden Baseline-Binary, Code-Revision/Patch-Hash, Toolchain, Build-Flags und Workload-Revision gesichert. Vergleiche alternieren die Reihenfolge; Build, Coverage und Fuzzing laufen nicht parallel. First-run-Kosten werden separat ausgewiesen, nicht still entfernt.

Vorgeschlagenes Regression-Gate: mindestens 15 A/B-Paare auf festgelegter Hardware, Median und Verteilung der gepaarten Verhältnisse sowie p95-Latenz und Peak Memory berichten. Eine reproduzierbare Verschlechterung von mehr als 3 % bei Durchsatz oder 5 % bei Peak Memory verlangt Begründung und Freigabe; statistisch unklare Läufe gelten nicht als Gleichheitsbeweis. Die Grenzen sind Projektvorschläge und müssen anhand der gemessenen Teststreuung festgeschrieben werden.

Referenzvergleiche mit Produktionsengines nennen Interpreter-/JIT-Modus, Warm-up, Features und Limits. Ein dauerhaft gespeicherter Workload-Manifest legt absolute Ziele fest. Solange diese Zielwerte fehlen, bleibt die Anforderung „performant“ unbewiesen. Der vorhandene [Performance-Bericht](jrs-performance.md) ist Ausgangsmaterial, kein globaler Nachweis.

Fuel ist nicht gleich reale CPU-Zeit. Der bestehende Opcode-Zähler kann während der Migration erhalten bleiben. Für optimierte Formate wird eine versionierte Kostenmetrik definiert, die Schleifen, native Scans, Compilation, GC und erzeugte Daten berücksichtigt. Eine Optimierung darf ihren Erfolg nicht dadurch erzielen, dass sie weniger Arbeit abrechnet oder Fuel-Abbrüche als Konformitätspässe deklariert.

## 16. Testarchitektur und Abnahmebeweise

### 16.1 Testebenen

| Ebene | Inhalt | Erwarteter Nachweis |
|---|---|---|
| Reine Unit-Tests | UTF-16, Numbers, Unicode, Regex, Queues, Bytecode-Verifier. | Grenzfälle, deterministische Ergebnisse, explizite Fehler. |
| Model-/Property-Tests | Descriptors, sparse Arrays, Shapes, Iterator-Zustände, Scheduler, Ephemerons. | Vergleich mit unabhängigem kleinen Modell, nicht mit derselben Implementierung. |
| Semantiktests | Konvertierungsreihenfolge, Proxy-Traps, Species, Completion und Reentry. | Beobachtbare Trace-Reihenfolge, Identität und Teiländerungen nach Fehlern. |
| GC-/Memory-Stress | Collection an jedem Safe Point, kleine Headrooms, Allocation-Fault-Injection. | Keine verlorenen Roots, stale Handles, Budget-Leaks oder unbegrenzten Queues. |
| Differential-Tests | Interpreter vs optimierter Pfad; zusätzliche externe Oracles. | Bitgenaue beziehungsweise spezifikationsgerechte Ergebnisse samt dokumentierten Ausnahmen. |
| Conformance | Original-Test262 und WPT mit gepinnten Inputs. | Vollständiges Inventar, echte Varianten/Completion, richtige Phase und Fehlerart. |
| Systemtests | OS-Adapter, Prozessisolation, I/O-Fehler, Shutdown, Clock-/Task-Verhalten. | Reale E2E-Ausführung einschließlich QEMU, wo relevant. |
| Performance | Versionierter Workload-Katalog. | Reproduzierbare A/B- und absolute Budget-Nachweise. |

Bestehende Coverage-Gates von 91 % Lines und 86 % Branches werden nicht abgesenkt. Sie sind Untergrenzen, keine Konformitätsgarantie. Tests dürfen nicht nur aus einem kurzfristigen, ignorierten `target/`-Script bestehen: Eine gefundene Semantikabweichung wird zum dauerhaften Regressionstest unter `src/tests/`.

### 16.2 Fuzzing

Unabhängige Targets prüfen Frontend, Bytecode-Verifier, Objekt-Operationen, Iteratoren, Scheduler, Regex und Buffer. Source-Fuzzing wird durch strukturierte gültige Programme ergänzt; andernfalls dominiert das schnelle Verwerfen ungültiger Syntax.

Für jedes reproduzierbare Versagen werden Seed, Mutation, Feature-Profil, Input, Kostenstand und minimierter Regressionfall gespeichert. Termination ist ein erlaubtes Ergebnis unter Limits, eine Invariant-Verletzung niemals. Safe-Rust-Code wird weiterhin mit Miri und, wo verfügbar, Sanitizern geprüft; späterer JIT-/FFI-Code bekommt zusätzliche Targets.

### 16.3 Vertrauenswürdige Runner

Runner dürfen keine Assertions, Testergebnisse oder benötigten APIs nachbilden, um einen Pass zu erhalten. Eigene Transport-Fixtures sind zulässig, aber keine Conformance-Tests.

Pflichtfälle für die Runner selbst sind fehlendes `done`, verspätete Assertion, doppelte Completion, falsche Fehlerphase, Harness-Exception, Fuel-Abbruch, leere Testauswahl, Timeout, Async-Job nach Scriptende und manipulierte beziehungsweise nicht aufrufbare Callback-Properties. Ein leerer Lauf ist nie grün.

Test262 benutzt frische Realms, Original-Harness-Includes, die vorgeschriebenen Strictness-Varianten, korrekte Modul-Fixtures und echte `$262`-Semantik. Sprach-Exceptions, fehlende Features, Harnessfehler und Resource-Abbruch werden getrennt ausgewiesen. Ein schneller Diagnose-Lauf mit niedrigem Fuel bleibt neben einem Konformitätslauf mit ausreichend hohem, dokumentiertem Budget bestehen.

WPT weist die getestete Umgebung aus: Shell ist weder Window noch DedicatedWorker. Soweit die vollständige Abnahme diese Umgebungen verlangt, müssen sie tatsächlich ausgeführt werden. Fail-/Unsupported-/Not-run-Zahlen bleiben vollständig sichtbar. Eine kuratierte Smoke-Auswahl darf nicht zur gesamten Suite umbenannt werden.

Referenzengines sind keine höhere Autorität als die gepinnte Spezifikation. Bei einer Abweichung werden lokaler Standardtext, kleiner Reproducer und mehrere geeignete Oracles verglichen. Eine Ausnahme braucht dokumentierte Begründung; sie darf nicht lediglich einen fehlgeschlagenen Vergleich verstecken.

## 17. Migrationsplan mit überprüfbaren Gates

Jede Phase besteht aus kleinen Änderungen. Das bestehende `jrs`-API bleibt zunächst als Fassade erhalten. Eine Änderung der Semantik wird getrennt von reinem Verschieben oder Umbenennen geprüft.

| Phase | Arbeit und Liefergegenstand | Exit-Kriterium |
|---|---|---|
| M0 – Baseline und Entscheidungen | Maschinenlesbares Feature-/Spec-/Testinventar; reproduzierbare Benchmarks; die beschlossene `regex-bt`-Nutzung als ADR dokumentieren, Webscope und Performance festlegen. | Testauswahl und Abnahmeumfang sind unverändert sichtbar. Binaries, Revisionen und Limits jedes Referenzlaufs sind gespeichert. Verbleibende offene Zielkonflikte sind nicht als erledigt markiert. |
| M1 – Semantischer Vertrag | Completion-/Termination-Typen, Script-/Realm-API angleichen, Effekte und interne Property-Schnittstellen festlegen. | Identische Scripts besitzen über Shell, Realm und Embedding dieselbe Global-Semantik; Phasen-/Exception-/Runner-Negativtests bestehen. |
| M2 – Source und Code entkoppeln | Source-, Frontend-, Bytecode- und Compiler-Grenzen extrahieren; UTF-16-Source und Verifier einführen. | CodeBlocks enthalten keine Heap-Identitäten; Parser-/Verifier-Fuzzing besteht; Source-Roundtrips und unterschiedliche Runtime-Budgets sind geprüft. |
| M3 – Objektmodell und Roots | Einheitliche Function-Objekte, typisierte Payloads, Intrinsic-Registry und Root-Scopes migrieren. | Keine Constructor-Schattenobjekte mehr; jede Objektart hat Descriptor-/Brand-/Trace-Tests; GC an jedem Safe Point ist grün. |
| M4 – Agent mit mehreren Realms | Heap-Owner von Realm trennen, Realm-IDs und Cross-Realm-Call-/Intrinsic-Verhalten implementieren. | `$262.createRealm`, gegenseitige Objektidentität, Fehler-Realm, Species-Fallbacks und Realm-Lifetime sind gezielt verifiziert. |
| M5 – Fortsetzbare Builtins | Callback-/Getter-/Proxy-fähige Native-Pfade auf explizite Frames und zentrale Dispatch-Schleife migrieren. | Tiefe JS/Builtin/Host-Ketten wachsen nicht auf dem Rust-Stack; Await/Generator-/Exception-Reentry teilt dieselben Frame-/Budget-Regeln. |
| M6 – Sprachfundament vervollständigen | Proxy, BigInt, Buffer/TypedArrays, vollständige Bindings, eval, Module und Generatoren auf den neuen Verträgen implementieren; `regex-bt` explizit anbinden und fehlende RegExp-Semantik vervollständigen. | Vollständige jeweilige Testfamilien einschließlich Fehlerpfaden, GC und Ressourcenfällen bestehen; RegExp-Arbeit teilt die Agent-Budgets. Verbleibende Familien bleiben sichtbar. |
| M7 – Host/Web ausgliedern | Vorhandene Events und Timer nach `jrs-web`; Native-State-Tracing, I/O-Requests und OS-Adapter stabilisieren. | Der ECMAScript-Core benötigt keine Web-Typen. Bestehende WPT-/E2E-Fälle bestehen über den neuen Adapter, inklusive Cancellation und Shutdown. |
| M8 – Interpreter optimieren | Shapes/Elements, Inline Caches, Code-/Value-Layout und gegebenenfalls neues Lowering einführen. | Optimized-on/off-Differential, Cache-Invalidation und Performance-Gates bestehen. Keine neue Semantikimplementierung nur für Fast Paths. |
| M9 – Webplattform ausbauen | Browser-Subsysteme und echten WPT-Produktadapter entlang ihrer Dependencies implementieren. | Jede neu behauptete Umgebung hat reale WPT-Ausführung. DOM-/Origin-/Lifecycle-/Rendering-Lücken sind nicht durch Shell-Pässe verdeckt. |
| M10 – Release-Audit | Gesamtes ursprüngliches Requirements-Inventar gegen finale Artefakte prüfen. | Alle vereinbarten vollständigen Konformitäts-, Sicherheits-, Abhängigkeits- und Performance-Nachweise liegen vor; offene Konflikte verhindern die vollständige Fertigmeldung. |

M6 kann in unabhängigen Feature-Strängen bearbeitet werden, sobald die jeweils benötigten Verträge stabil sind. M8 beginnt mit frühem Profiling, aktiviert seine strukturellen Optimierungen aber erst nach M3/M5. M7 braucht für Cross-Realm-Webobjekte M4. M9 ersetzt nicht die noch offene ECMAScript-Vervollständigung.

Termine werden erst nach M0/M1 und einem gemessenen Pilotumbau geschätzt. Bestehende Pass-Zahlen sind keine Aufwandsschätzung. Vollständige Sprach- und Browser-Funktionalität ist kein seriös zusagbares Nebenprodukt einer Folge kleiner Builtin-Erweiterungen.

## 18. Erster umsetzbarer Arbeitsauftrag nach Freigabe

Der nächste Architektur-Arbeitsschritt sollte keine neue große Sprachfunktion sein, sondern ein begrenzter Pilot:

1. Ein strukturiertes Inventar der öffentlichen APIs, Heap-Roots, Realm-Zustände und Native-Reentry-Stellen erzeugen.
2. Eine gemeinsame Outcome-/Completion-Grenze definieren und an einem vorhandenen Builtin-Familienpfad testen.
3. `Runtime::run` und persistente Script-Ausführung auf dieselbe Global-Instantiation ausrichten; bisherige Unterschiede durch Regressiontests festhalten.
4. Einen Constructor vollständig auf echte Function-Objekte statt Property-Schattenstorage migrieren, einschließlich Mutation, Alias, Species und Realm-Zuordnung.
5. Vorher-/Nachher-Konformität, GC-Stress, Allocation-Zahlen und Performance vergleichen; erst dann den Vertrag auf weitere Constructoren ausrollen.

Der Pilot ist fertig, wenn der gemeinsame Vertrag funktioniert und die bestehenden Tests ohne verdeckte Ausnahmen weiterlaufen. Er ist nicht gleichbedeutend mit dem Abschluss des gesamten Goals.

## 19. Review- und Änderungsregeln

Für jedes Feature oder jede Optimierung beantwortet der Review:

- Welcher lokale Standardabschnitt und welche Version begründen das Verhalten?
- Wer besitzt Source, Resultate, temporären Zustand und externe Ressourcen?
- Welche Schritte dürfen JS ausführen, allokieren oder suspendieren?
- Welche Werte bleiben über diese Schritte hinweg rooted?
- Welche Properties und Prototypes können sich währenddessen ändern?
- Welche Teiländerungen müssen nach einer Exception sichtbar bleiben?
- Welche Budgets verhindern unbegrenzte Arbeit beziehungsweise Memory-Wachstum?
- Welche unabhängigen Tests widerlegen eine plausible fehlerhafte Implementierung?
- Wie wird das Ergebnis mit ausgeschalteten Optimierungen verglichen?
- Welche Performance-/Konformitätsaussage ist tatsächlich durch den Lauf gedeckt?

Neue Crates werden in Layering-, Dependency-, Coverage-, Fuzz- und Dokumentationskatalog aufgenommen. Externe Downloads erfolgen nur als explizite Daten-/Testaktualisierung. Die beschlossene `regex-bt`-Nutzung sowie Entscheidungen zu JIT, Shared Memory und Browser-Integration bekommen eigene ADRs mit Alternativen, Konsequenzen und Verifikation. Für `regex-bt` dokumentiert die ADR die erteilte Freigabe und den Budget-Vertrag; sie stellt die Backend-Wahl nicht erneut als offen dar.

## 20. Definition of Done

Eine vollständige Fertigmeldung ist erst zulässig, wenn alle folgenden Punkte belegt sind:

- [ ] Die widersprüchlichen beziehungsweise unbestimmten Produktanforderungen aus Abschnitt 1 sind ausdrücklich entschieden, nicht still eingeschränkt.
- [ ] Ein Scope-/Spec-/Testmanifest benennt jede Anforderung und den zugehörigen Nachweis.
- [ ] Sämtliche verlangten Test262-Tests und Varianten bestehen unter dokumentierter, ausreichender Ressourcenkonfiguration; Diagnose-Unsupported und fehlende Harness-Funktionen zählen nicht als Pass.
- [ ] Die verlangten WPT-Umgebungen und Testtypen wurden vollständig mit dem echten Produktadapter ausgeführt. Für die ursprüngliche Forderung nach vollständigem WPT genügt keine ausgewählte Shell-Liste.
- [ ] Core und benötigte Adapter bauen ohne externe Software-Abhängigkeiten und ohne verdeckte Download-/Fallback-Pfade.
- [ ] jrs verwendet das freigegebene `regex-bt`-Backend; Matcher-, Compilation- und Speicherbudgets sind integriert, auditiert und durch Worst-Case-Tests geprüft. Es wird keine allgemeine Linearitäts- oder vollständige ReDoS-Garantie behauptet.
- [ ] GC-, Handle-, Proxy-, Buffer-, Realm-, Task- und Completion-Invarianten sind durch Tests und Audits gedeckt.
- [ ] Reproduzierbare Performance-, Latenz- und Memory-Ziele sind auf den festgelegten Zielsystemen erfüllt.
- [ ] Full Check, Coverage-Gates, Miri, relevante Systemtests und Fuzz-Replay bestehen am exakt ausgelieferten Stand.
- [ ] Bekannte Einschränkungen, offene Tests und Fehler sind vollständig erfasst; keine unzulässige Ausnahme ist als „nicht relevant“ aus der Abnahme verschwunden.

Grüne lokale Gates beweisen die Qualität des geprüften Ausschnitts. Sie beweisen weder allein vollständiges ECMAScript noch allein eine vollständige Webplattform.

## 21. Quellen und Geltungsbereich dieses Dokuments

Gelesene lokale Grundlagen:

- [ECMA-262-Provenienz](ecma/README.md) und die lokale Fassung, insbesondere Realms, Execution Contexts, Jobs/Agents sowie gewöhnliche und Proxy-Internal-Methods.
- [Code-Organisation](05-code-organization.md), [Teststrategie](06-testing-strategy.md) und die Coverage-/Layering-Policy in [xtask](../crates/tools/xtask/src/policy.rs).
- [jrs-Core](../crates/jrs/src/lib.rs), [VM](../crates/jrs/src/vm.rs), [Realm-API](../crates/jrs/src/vm/realm.rs), [Heap](../crates/jrs/src/heap.rs), [Object](../crates/jrs/src/object.rs), [Values](../crates/jrs/src/value.rs) und [Compiler/Bytecode](../crates/jrs/src/bytecode.rs).
- [Test262-Einbindung](test-ext/README.md), [Host-Standard-Provenienz](whatwg/README.md) und [Performance-Messungen](jrs-performance.md).

Der Entwurf ist keine Implementierungs- oder Vollständigkeitsbehauptung und keine Übernahme des Codes einer anderen Engine. Bestehende Spezialpläne, etwa zu Browser- oder WebCrypto-Integration, werden durch dieses neue Dokument nicht verändert; sie müssen bei ihrer Umsetzung mit den hier vorgeschlagenen Grenzen abgeglichen werden.
