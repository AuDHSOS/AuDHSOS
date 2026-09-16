<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Manuel Baesler and contributors -->

# jrs: Zielarchitektur und Migrationsplan

Stand: 9. September 2026, ergänzt am 13. September 2026 um die Migrationsrichtung in Abschnitt 17.1. Status: Architekturvorschlag, noch keine Implementierungsfreigabe für sämtliche beschriebenen Erweiterungen.

Dieses Dokument beschreibt eine wartbare, erweiterbare, testbare und auf hohe Performance ausgerichtete Architektur für jrs. Es basiert auf einer lesenden Prüfung des vorhandenen Workspaces. Es verändert nicht dessen laufende Implementierung. Die Konformitäts- und Performance-Ziele bleiben bestehen; die frühere Beschränkung auf eine reine Automaton-Engine ist durch die ausdrückliche Freigabe von `regex-bt` ersetzt. Vorgeschlagene Crates, Typen und APIs sind ausdrücklich Zielzustand, sofern sie nicht als vorhanden bezeichnet werden.

Die Architekturentscheidung lautet: **ein semantisch einheitlicher ECMAScript-Core mit klarer Agent-/Realm-Zuordnung, einem gemeinsamen Objektmodell, expliziten Ausführungsfortsetzungen und getrennten Host-/Webplattform-Adaptern**. Optimierungen setzen auf diesen Verträgen auf und dürfen keine zweite, abweichende Sprachsemantik etablieren.

Dieser Core entsteht im registerbasierten Engine-Pfad unter `crates/jrs/src/engine/`. Das Stack-Backend unter `crates/jrs/src/vm.rs` ist die Quelle der zu übernehmenden Semantik und wird vollständig stillgelegt, nicht dauerhaft gepflegt. Dass beide Pfade heute nebeneinander existieren, ist ein befristeter Migrationszustand mit den Regeln aus Abschnitt 17.1, kein Architekturziel.

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

1. **Eine Semantik:** Script, Module, `eval`, dynamische Function-Constructors, Host-Aufrufe und optimierte Ausführung benutzen dieselben abstrakten Operationen und Completion-Regeln. Jede Klausel hat am Ende genau eine Implementierung; während der Migration gilt Abschnitt 17.1.
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

Die Ausführung des Ziel-Cores ist der registerbasierte Interpreter unter `crates/jrs/src/engine/`. Das bestehende Stack-Bytecode ist der Migrationsstart und die Quelle der zu übernehmenden Semantik, nicht das Ziel: Jede Familie wird dorthin überführt und danach aus `vm.rs` entfernt. Dauerhaft gepflegt wird ein produktives Format, nicht eine beliebig wachsende Sammlung gleichberechtigter Interpreter.

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
| M8 – Interpreter optimieren | Shapes/Elements, Inline Caches sowie Code-/Value-Layout im Engine-Core ausbauen und messen. | Optimized-on/off-Differential, Cache-Invalidation und Performance-Gates bestehen. Keine neue Semantikimplementierung nur für Fast Paths. |
| M9 – Webplattform ausbauen | Browser-Subsysteme und echten WPT-Produktadapter entlang ihrer Dependencies implementieren. | Jede neu behauptete Umgebung hat reale WPT-Ausführung. DOM-/Origin-/Lifecycle-/Rendering-Lücken sind nicht durch Shell-Pässe verdeckt. |
| M10 – Release-Audit | Gesamtes ursprüngliches Requirements-Inventar gegen finale Artefakte prüfen. | Alle vereinbarten vollständigen Konformitäts-, Sicherheits-, Abhängigkeits- und Performance-Nachweise liegen vor; offene Konflikte verhindern die vollständige Fertigmeldung. |

M6 kann in unabhängigen Feature-Strängen bearbeitet werden, sobald die jeweils benötigten Verträge stabil sind. M8 beginnt mit frühem Profiling, aktiviert seine strukturellen Optimierungen aber erst nach M3/M5. M7 braucht für Cross-Realm-Webobjekte M4. M9 ersetzt nicht die noch offene ECMAScript-Vervollständigung.

Termine werden erst nach M0/M1 und einem gemessenen Pilotumbau geschätzt. Bestehende Pass-Zahlen sind keine Aufwandsschätzung. Vollständige Sprach- und Browser-Funktionalität ist kein seriös zusagbares Nebenprodukt einer Folge kleiner Builtin-Erweiterungen.

### 17.1 Migrationsrichtung und Stilllegung des Stack-Backends

Zielzustand: jrs führt jedes Programm über den Engine-Core aus. `crates/jrs/src/vm.rs` und die dort liegenden Builtins sind entfernt, nicht deaktiviert.

Weg dorthin: Die Phasen M1 bis M7 werden im Engine-Core gebaut, nicht mehr im Stack-Backend. Vorhandene Semantik wird dabei übernommen, wo sie brauchbar ist; sie wird gelesen, portiert und gegen den alten Pfad geprüft, nicht neu erraten. Was nicht übernehmbar ist, wird als solches benannt und neu implementiert.

Je Familie in dieser Reihenfolge:

1. Den entsprechenden Code im Stack-Backend lesen und die Semantik, Auswertungsreihenfolge, Fehlerfälle, GC-Regeln und Budgets übernehmen.
2. Differential gegen den alten Pfad, bis beide für jedes Programm, das beide ausführen können, dasselbe Ergebnis liefern.
3. Fokussierter Test262-Lauf der Familie.
4. Den alten Pfad entfernen, sobald ihn kein Ausführungsweg mehr erreicht.
5. Vollständiger Test262-Lauf.

Regeln für den Zwischenzustand, solange beide Pfade existieren:

- **Gleichheitspflicht:** Für jedes Programm, das beide Pfade ausführen können, liefern sie dasselbe Ergebnis. Eine Abweichung ist ein Fehler und wird vor der nächsten Familie behoben, nicht dokumentiert und stehengelassen.
- **Kein einseitiger Zuwachs:** Ein Feature, das der Stack-Pfad nicht hat, wird nicht allein im Engine-Core ergänzt, solange der Stack-Pfad noch Programme ausführt. Sonst hängt das Verhalten davon ab, welcher Pfad das Programm kompiliert hat.
- **Befristete Doppelung:** Die Doppelung einer Familie endet mit dem Migrationsschritt, der sie überführt. Eine Familie bleibt nicht dauerhaft in beiden Pfaden.
- **Ein Einstieg:** `Runtime::run`, `Realm::evaluate` und das Embedding benutzen denselben Lowering-Pfad. Ein Ausführungsweg, den der Engine-Core nicht erreicht, ist eine Migrationslücke und wird als solche geführt.
- **Die Lücke gehört an ihre Stelle:** Eine Regel, die die Grenze durchsetzt, wird nicht zusätzlich im Lowering geprüft. Sonst meldet der Engine-Core "ein Script, das das Lowering nicht nimmt", wo er sagen könnte, was genau fehlt. Umgekehrt gilt: `Unsupported` vergiftet den Realm, weil sein Zustand danach unbekannt ist — außer bei einem Wert, der die Grenze nicht überqueren kann. Dort ist das Script an einem definierten Ende angekommen, und nur sein Wert fehlt.
- **Lücke statt Antwort:** Was der Engine-Core noch nicht gebaut hat, wird als `Unsupported` benannt und nie als Wert beantwortet. Ein Name, den eine noch nicht gebaute Intrinsic besäße, ist deshalb kein `undefined`: die Namenslisten von Klausel 19, 20.1.3, 22.1.3 und 23.1.3 liegen im Realm, damit ein Fehltreffer die Lücke nennt. Eine Lücke kostet einen Test, eine falsche Antwort kostet das Vertrauen in jede Zahl.

Die Doppelung ist damit ein Zustand mit Ablaufdatum, kein paralleler Semantikpfad im Sinn von Abschnitt 3 Regel 1.

#### Erste Meilensteingruppe: Realm-Zustand im Engine-Core

Der Engine-Core besitzt heute einen eigenen Heap und ein eigenes Objektmodell,
die vom Realm des Stack-Backends getrennt sind. Über diese Grenze kommen nur
Primitive: `register_primitive` kennt undefined, null, Boolean, Number und
String, kein Objekt. Deshalb kann der Engine-Core keinen Realm-Zustand halten,
deshalb lehnt `register_script_features` im Realm-Modus jede Deklaration ab,
und deshalb erreicht ihn keine Test262-Datei. Die Reihenfolge folgt daraus:

| Schritt | Inhalt | Exit-Kriterium |
|---|---|---|
| G0 | Eine Operation kann Benutzercode aufrufen und danach weiterlaufen: explizite Builtin-Frames mit Algorithmusphase nach Abschnitt 8, zuerst für ToPrimitive (7.1.1). | `1+{valueOf(){return 2}}` antwortet im Engine-Core wie im Stack-Backend. |
| G1 | Globales Objekt und Global Environment Record (9.1.1.4) im Engine-Realm; `globalThis`; ReferenceError für nicht auflösbare Namen. | Ein nicht deklarierter Name wirft dieselbe ReferenceError wie im Stack-Backend. |
| G2 | `LdaGlobal`/`StaGlobal` mit Property-Cell-Caches und Invalidation. | Differential gegen den Stack-Pfad über Lesen, Schreiben, Löschen und Shadowing. |
| G3 | GlobalDeclarationInstantiation (16.1.7) für `var`, `function`, `let`, `const` samt Redeklarationsfehlern. | Bindungen überleben mehrere `Realm::evaluate`-Aufrufe; Fehlerfälle von 16.1.7 sind geprüft. |
| G4 | Backend-Auswahl pro Realm statt pro Script. | Ein Realm läuft vollständig auf einem Pfad; ein Programm kann nicht mehr davon abhängen, welcher Pfad es kompiliert hat. |
| G5a | Code-Identität gehört dem Realm: der Realm hält den Code jedes Scripts, das er ausgeführt hat, und ein Funktionsobjekt nennt seine Unit. Erledigt. | `function f(){}` in einem Script, `f()` im nächsten desselben Realms antwortet wie im Stack-Backend. |
| G5b | Das Lowering nimmt einen Aufruf eines globalen Namens auch dort, wo sein Ergebnis statisch nicht typisierbar ist: als Rückgabewert und im Rumpf einer Schleife, deren Kopf dafür von den Typen der Zuweisungen des Rumpfes ausgeht. Erledigt. | Ein Aufruf eines globalen Namens wird an jeder Stelle übersetzt, an der der Stack-Pfad ihn ausführt. |
| G5c | `this` ist der Receiver des Aufrufs (10.2.1.2), und ein Methodenaufruf erreicht eine Funktion des Scripts, nicht nur eine Intrinsic. Erledigt. Ein Aufruf ohne Receiver ist eine Lücke, bis die Code-Unit die Strictness nennt; ein Arrow mit `this` wird abgelehnt (10.2.1.1). | `let o={a:1,g:function(){return this.a}};o.g()` antwortet wie im Stack-Backend. |
| G5d | Eigenschaften eines Werts, den das Lowering nicht benennen konnte, werden gelesen und geschrieben; ein Funktionsobjekt trägt eigene Properties. Erledigt. | `var f=function(){};f.z=1;f.z` antwortet wie im Stack-Backend. |
| G5e | `new`: `[[Construct]]` (10.2.2), `OrdinaryCreateFromConstructor` (10.1.13) und die `prototype`-Property eines Funktionsobjekts (10.2.5). Erledigt. | `function F(a){this.x=a}new F(41).x` antwortet wie im Stack-Backend. |
| G5g | `OrdinaryCallBindThis` (10.2.1.2): ein Aufruf ohne Receiver bindet `this` an das globale Objekt, wenn die Funktion nicht strict ist, und lässt es undefined, wenn sie es ist. Erledigt. | `function f(){return typeof this}f()` antwortet wie im Stack-Backend. |
| G5f | `instanceof` (13.10.2, `OrdinaryHasInstance` 7.3.22); Konstruktor und Methode eines Realms werden zur Laufzeit aufgelöst, weil eine Funktionsdeklaration dort ein Name des Global Environment Record ist; das Werfen und das Completion eines Objects gehören an die Grenze, nicht ins Lowering. Erledigt. | `harness/sta.js` wird vollständig übersetzt. |
| G5h | Das Completion eines Scripts, das für seine Wirkung läuft, muss die Grenze nicht überqueren: `Realm::run_compiled` verwirft es, und der Test262-Runner nimmt es, weil ein Verdikt am Geworfenen hängt und nie am Wert. Erledigt. | `harness/sta.js` läuft im Engine-Core vollständig durch. |
| G5i | Ein Parameter ist ein beliebiger Wert (10.2.11). Ein Object, das an einen Aufruf geht, verliert sein Layout, denn der Aufgerufene erreicht es; eine Funktion nimmt ihre Closure mit. Eine Konvertierung, die ToPrimitive bräuchte, nennt die Lücke an ihrer Stelle statt zu antworten. Erledigt. | `harness/assert.js` läuft im Engine-Core; die Test262-Zahlen des Pfades sind messbar. |
| G5j | `for`-`in` nimmt jeden Head (14.7.5.6: undefined und null zählen nichts auf, ein Primitive braucht ToObject und nennt die Lücke zur Laufzeit) und die Bindung, die eine `var`-Deklaration gemacht hat (14.7.5.5, 8.2.7). Erledigt. | `function f(o){var s="";for(var k in o){s+=k}return s}` antwortet wie im Stack-Backend. |
| G5k | `arguments` (10.4.4): das unmapped Arguments-Object (10.4.4.7) mit Index-Properties, `length` und `callee`; 10.2.11 bindet den Namen. Ein Rumpf, der auch einen Parameter schreibt, könnte das Mapping sehen, das dieser Core nicht baut, und wird nicht übersetzt. Erledigt. | `function f(){return arguments.length}f(1,2)` antwortet wie im Stack-Backend. |
| G5l | `delete` (13.5.1.2) und `[[Delete]]` eines Ordinary Objects (10.1.10.1): die Shape ohne den Namen, der Element-Store und das `length` eines Arrays (10.4.2). Ein Name, den eine Deklaration gebunden hat, antwortet false (9.1.1.1, 16.1.7); ein freier Name gehört dem globalen Objekt und bleibt eine Lücke. Erledigt. | `var o={a:1};delete o.a` antwortet wie im Stack-Backend. |
| G5m | Ein Property-Write unter einem Key, den erst die Laufzeit kennt (13.15.2, `PutValue` 6.2.5.5, `OrdinarySetWithOwnDescriptor` 10.1.9.2): `SetByValue` nennt die Lücke dort, wo der Store läuft, wenn der Name einem Prototype gehört, den dieser Realm nicht gebaut hat. Ein berechneter Key eines Literals definiert (13.2.5.5) und erreicht keinen Prototype. Erledigt. | `let f=function(o,k){o[k]=1;return o[k]};f({},'a')` antwortet wie im Stack-Backend. |
| G5n | Der Join von 14.6.2 nimmt die Objects, die seine Zweige gemacht haben: eines, das nur ein Zweig macht, behält sein Layout; eines, das die Zweige verschieden formen, gibt es auf, und ein Wert, dessen Layout der Join nicht halten konnte, gibt auch seinen Typ auf. Erledigt. | `harness/propertyHelper.js` wird vollständig übersetzt. |
| G5o | `%Array%` (23.1.1.1) auf dem globalen Objekt: eine Länge oder viele Elemente, `IsArray` (23.1.2.3), und die Kopplung von Konstruktor und `%Array.prototype%` (23.1.2.5, 23.1.3.2). `new Array(...)` erreicht dieselbe Funktion, weil ein nativer Konstruktor sein eigenes Object antwortet. Ein Name, den 23.1.2 gibt und dieser Realm nicht gebaut hat, ist eine Lücke. Erledigt. | `new Array(3).length` antwortet wie im Stack-Backend. |
| G5p | `%Object%` (20.1.1.1) auf dem globalen Objekt: undefined und null machen ein Ordinary Object, jedes Object antwortet unverändert, ein Primitive nennt den Wrapper, den `ToObject` bräuchte. Ein Name, den 20.1.2 gibt und dieser Realm nicht gebaut hat, ist eine Lücke. Erledigt. | `typeof new Object()` antwortet wie im Stack-Backend. |
| G5q | `Object.defineProperty` (20.1.2.4), `Object.getOwnPropertyDescriptor` (20.1.2.8) und `Object.getOwnPropertyNames` (20.1.2.10), mit `FromPropertyDescriptor` (6.2.6.4) und `ToPropertyDescriptor` (6.2.6.5). Ein Feld, das ein Descriptor nicht trägt, ist absent (6.2.6.6). Ein Accessor darin ist eine Lücke, weil dieser Core keine Accessor-Property hat. Erledigt. | `var o={};Object.defineProperty(o,'x',{value:5});o.x` antwortet wie im Stack-Backend. |
| G5r | Ein Compound Assignment (13.15.2) an eine Property und an einen Namen des Global Environment Record: die Reference wird einmal ausgewertet, gelesen und mit dem Operator von 13.15.3 zurückgeschrieben. Erledigt. | `var o={a:1};o.a+=2;o.a` antwortet wie im Stack-Backend. |
| G5s | Eine lexikalische Deklaration eines Scripts bindet auf dem `[[DeclarativeRecord]]` des Global Environment Record (16.1.7): Schritt 3 prüft jeden Namen, bevor Schritt 16 einen anlegt, 9.1.1.4.4 gibt ihm seinen Wert dort, wo die Deklaration steht, und ein `const` bleibt unveränderlich (9.1.1.4.5). Erledigt. | `let x = 1;` und danach `x + 1` antworten im selben Realm wie im Stack-Backend. |
| G5t | `%Function%` (20.2.1.1) auf dem globalen Objekt und `Function.prototype.call` (20.2.3.3), das keinen Wert antwortet, sondern ruft: der Receiver wird der Callee, das erste Argument das `this`, der Rest rückt um eins. Der `Function`-Konstruktor selbst übersetzt einen Body zur Laufzeit und bleibt eine Lücke. Erledigt. | `function f(a){return this.v+a};f.call({v:1},2)` antwortet wie im Stack-Backend. |
| G5u | `Function.prototype.bind` (20.2.3.2) und das Bound Function Exotic Object (10.4.1): 10.4.1.1 ruft das Ziel mit dem gebundenen `this`, und 7.2.3 zählt es als callable. `[[BoundArguments]]` ist leer; ein gebundenes Argument müsste vor die Argumente des Aufrufs, die in Registern des Aufrufers liegen, und wird deshalb benannt. Erledigt. | `function f(){return this.v};f.bind({v:5})()` antwortet wie im Stack-Backend. |
| G5v | `%Math%` (21.3) als Namespace-Object des globalen Objekts, mit `Math.pow` (21.3.2.26). Ein Name, den 21.3 gibt und dieser Realm nicht gebaut hat, ist eine Lücke. Damit läuft `harness/propertyHelper.js` im Engine-Core vollständig durch. Erledigt. | `Math.pow(2,32)-1` antwortet wie im Stack-Backend, und der Harness lädt. |
| G5w | Die Error-Konstruktoren (20.5.1.1, 20.5.6.1.1) auf dem globalen Objekt, je unter dem Prototype ihres Konstruktors, mit `message` nur dann, wenn eines übergeben wurde. Die Fehler, die dieser Core wirft, entstehen durch dieselbe Operation, also fängt ein Script sie an ihrem Konstruktor. Erledigt. | `try{null.x}catch(e){e instanceof TypeError}` antwortet wie im Stack-Backend. |
| G5x | `%String%` (22.1.1.1) auf dem globalen Objekt: ein Aufruf antwortet den String, den `ToString` macht, ohne Argument den leeren, und 17 koppelt Konstruktor und `%String.prototype%`. `new` macht das String Exotic Object von 10.4.3, das dieser Core nicht gebaut hat, und nennt das als Lücke. Ein Property-Read auf einem String löst über `%String.prototype%` dort auf, wo er läuft, nicht im Lowering. Was dabei erreichbar wurde, nennt jetzt seine Lücke statt einer falschen Antwort: `ToString` eines Objects braucht `ToPrimitive` (7.1.1), 22.1.3 beginnt jede Methode mit `RequireObjectCoercible` und `ToString`, ein Name, den ein intrinsischer Prototype besitzt und dieser Realm nicht gebaut hat, ist auch auf dem Prototype selbst eine Lücke, und ein Property Key, den ein Script gemacht hat, wird interniert, weil eine Shape ihre Namen per Referenz hält. Erledigt. | `"".constructor===String` und `String.prototype.charAt.call(42,0)` antworten wie im Stack-Backend. |
| G5y | Jede Ablehnung des Lowerings nennt das Konstrukt, an dem sie steht, nach dem Namen der Grammatik; die innerste zuerst, weil ein äußerer Knoten nur scheitert, weil ein innerer es tat. Ohne das ist keine Reihenfolge der Arbeit ablesbar. `this` außerhalb einer Funktion ist der erste Fall: 9.4.2 löst es auf dem Environment Record auf, der eines hat, und auf oberster Ebene eines Scripts ist das der Global Environment Record mit `[[GlobalThisValue]]` (9.1.1.4.11). Erledigt. | `typeof this` antwortet wie im Stack-Backend, und `verifyProperty(this, ...)` erreicht den Harness. |
| G5z | Jede fehlende Intrinsic nennt das Objekt, das sie schuldet, weil daraus die Reihenfolge der Arbeit folgt. `%Object%` schuldet am meisten, und sechs Funktionen aus 20.1.2 sind gebaut: 20.1.2.2 und 20.1.2.3 (mit 20.1.2.3.2, das jeden Descriptor liest, bevor es einen definiert), 20.1.2.12, 20.1.2.19, 20.1.2.14 (`SameValue`, 7.2.11) und 20.1.2.13. Erledigt. | `Object.create({},{a:{value:5}}).a` und `Object.is(0,-0)` antworten wie im Stack-Backend. |
| G6a | Die acht Methoden aus 23.1.3, die ohne Callback auskommen: 23.1.3.27, 23.1.3.37, 23.1.3.31, 23.1.3.7, 23.1.3.4, 23.1.3.2, 23.1.3.39 und 23.1.3.33. Ein Loch bleibt ein Loch, wo die Klausel über `HasProperty` liest. Ein Receiver ohne den Elements-Store von 10.4.2 nennt die Lücke, statt einen kaputten Frame zu melden; sein `length` wird geschrieben, wie 7.3.4 jede Property schreibt. Erledigt. | `[1,2,3].splice(1,1)` und `[1,2,3].toReversed()` antworten wie im Stack-Backend. |
| G6b | Die neun Methoden aus 23.1.3, die das Script nach jedem Element fragen (23.1.3.6, .8, .9, .10, .15, .21, .24, .25, .29). Jedes Element ist ein Aufruf, also verlässt der Core die Methode und kommt über den Frame zurück, den der Aufruf geöffnet hat, wie 7.1.1 es für ein `valueOf` schon tat. Was der Lauf erreicht hat, überlebt diese Frames und liegt daher in einem Objekt des Heaps, das der Collector verfolgt, benannt von einem Root, den `Resume` trägt; der Callback nimmt seine Argumente von dort, geschrieben in das Fenster des Callee nach `bind_this`. Erledigt. | `[1,2,3].map(function(x){return x*2})` antwortet wie im Stack-Backend. |
| G6c | Die acht Werte von 21.3.1 und die Funktionen aus 21.3.2, die ohne Bibliothek für ein Transzendentes auskommen: Betrag, die drei Rundungen, Vorzeichen, die beiden Extrema und die drei auf 32-Bit-Integern. 6.1.6.1 unterscheidet die beiden Nullen, und jede Klausel sagt, welche sie antwortet. Erledigt. | Über `test/built-ins/Math` besteht der Core 118 von 654 Varianten und scheitert an keiner. |
| G6d | Ein Parameter nimmt seinen Initializer (8.6.2), wo der Aufruf undefined übergab, und 15.1.5 zählt das `length` von 10.2.9 bis zum ersten davon. Ein Realm-Script mit `let` oder `const` in einem Block wird genommen; was das Lowering nicht nimmt, nennt sich dort, wo es steht. Jeder Lauf über eine `length` aus 23.1.3 wird verrechnet, bevor er beginnt, weil ein Loch keinen Frame öffnet, an dem Fuel hinge. Erledigt. | `function f(a=1){return a};f()` antwortet wie im Stack-Backend, und `new Array(4294967294).fill(1)` endet an einer Ressourcengrenze. |
| G6e | Ein `for`-`of` läuft über ein Iterable nach dem Protokoll von 7.4: `GetWellKnown` liest `@@iterator`, 7.4.6 ruft `next` und fragt `done`, 7.4.7 liest `value` nur, wo es falsch ist. 27.1.2.1 gibt `%IteratorPrototype%` ein `@@iterator`, das `this` antwortet. Ein Symbol, das ein ungebauter Prototype besitzt, nennt seine Lücke, statt undefined zu antworten. Ein Körper, den `break` oder `return` verlässt, wird abgelehnt, weil 7.4.9 den Iterator schließen müsste. Erledigt bis auf 7.4.9. | `for(x of [1,2].values())` antwortet wie im Stack-Backend. |
| G6f | Ein Property Descriptor ändert, was eine Property ist, nicht nur was sie hält: die Shape trägt die Attribute mit den Namen, also gehört eine Property mit anderen Attributen zu einer anderen Shape. Darauf stehen die Integritätsstufen: 20.1.2.20 und 20.1.2.16 lesen und schreiben `[[Extensible]]`, 20.1.2.22 und 20.1.2.6 setzen die Stufe von 7.3.14, 20.1.2.18 und 20.1.2.17 prüfen sie mit 7.3.15. Dazu 20.1.2.24 und 20.1.2.5. Erledigt. | `Object.freeze(o)` und `Object.isFrozen(o)` antworten wie im Stack-Backend. |
| G6g | Ein Update nimmt `ToNumeric` des alten Werts (13.4.4.1). `%Number%` (21.1.1.1) auf dem globalen Objekt, mit den acht Werten und den vier Fragen aus 21.1.2, von denen keine konvertiert; `new` nennt das Number Exotic Object von 21.1.3 als Lücke. Erledigt. | `Number('42')` und `Number.isInteger(-3)` antworten wie im Stack-Backend. |
| G6h | Ein unabgefangenes `throw` eines Objects, das die Einbettung nicht halten kann, ist keine fehlende Funktion: das Script lief bis zu einem `throw`, und das ist eine Completion der Sprache. Die Grenze antwortet, dass geworfen wurde, nicht was. Erledigt. | `throw {}` meldet `ThrownUnrepresentable`, und der Realm bleibt benutzbar. |
| G6i | `%Boolean%` (20.3.1.1) und `%Reflect%` (19.4.4, 28.1): neun Operationen aus 28.1, die dieselben sind wie in 20.1.2, nur ohne deren Coercion und mit einer Antwort statt eines Wurfs. Was 28.1 sonst noch gibt, nennt seine Lücke — was 206 Varianten kostet, die nur bestanden, weil `Reflect.construct` fehlte und der Harness den TypeError schluckte. Erledigt. | `Reflect.has({a:1},'a')` antwortet wie im Stack-Backend. |
| G6j | Accessor Properties (6.1.7.1): Der Slot hält das Paar, 10.1.8.1 und 10.1.9.2 rufen es auf und verlassen die Instruktion wie eine Konversion aus 7.1.1. 10.1.6.3 ist die Operation, die die Spezifikation schreibt, nicht mehr ein Zusammenlegen von Attributen. Drei Lücken bleiben benannt: ein Native, das einen Accessor findet; ein Setter, der kein Script ist; und 10.4.2.1. Erledigt. | `Object.defineProperty(o,'x',{get(){return 1}});o.x` antwortet wie im Stack-Backend. |
| G6k | 10.4.2.1: Ein Index, der als gewöhnliche Data Property herauskommt, steht im Element-Store; jeder andere verlässt ihn und wird eine Property der Shape. Ein Read geht über ein Loch hinweg weiter (10.1.8.1), ein Write sieht zuerst in der Shape nach. Die `length` eines Arrays bleibt Lücke (10.4.2.4). Erledigt. | `Object.defineProperty([1,2],'1',{value:9,writable:false});a[1]` antwortet wie im Stack-Backend. |
| G6l | Ein Native, das ein Argument durch 7.1.17 schickt, bevor es sonst etwas tut, verlässt den Aufruf wie eine Konversion aus 7.1.1 eine Instruktion verlässt und läuft von vorn, sobald das Register des Aufrufers ein Primitive hält. 7.1.1 nimmt zum ersten Mal seinen Hint. `%String%` und die sieben Error-Konstruktoren fragen so. Erledigt. | `String({})` und `new Error({toString(){return 'x'}})` antworten wie im Stack-Backend. |
| G6m | Die Wrapper-Objekte aus 21.1.3 und 20.3.3, mit den vier Methoden, die ihre Daten zurückgeben (21.1.3.7, 21.1.3.6, 20.3.3.3, 20.3.3.2). 7.1.18 gibt jedem Wrapper das Prototype seines eigenen Konstruktors. Eine andere Basis als 10 bleibt Lücke. Erledigt. | `new Number(1).valueOf()` und `String(new Boolean(0))` antworten wie im Stack-Backend. |
| G6n | Das String Exotic Object aus 22.1.4 und 10.4.3: Indizes und `length` kommen aus den `[[StringData]]`, nicht aus einer Shape. 22.1.3.32 und 22.1.3.28 sind gebaut. Eine Methode aus 22.1.3 auf einem Wrapper bleibt Lücke, weil 7.1.17 des Receivers ein Aufruf ist. Erledigt. | `new String('abc')[1]` und `Object.keys(new String('ab'))` antworten wie im Stack-Backend. |
| G6o | 13.12, 13.9 und 13.11.1 nehmen dieselbe Instruktion wie die arithmetischen Operatoren, die einen Object-Operanden konvertiert — im Ausdruck wie in der zusammengesetzten Zuweisung. 7.2.14 behält, was es nicht konvertiert. `Unknown` gilt nicht mehr als Primitive, wenn die typisierte Form gewählt wird. Erledigt. | `new Number(1) \| true` und `({}) == null` antworten wie im Stack-Backend. |
| G6p | 13.2.5.1: Eine Instruktion definiert eine Accessor-Property eines Literals und lässt die andere Hälfte stehen, so dass `get` und `set` desselben Namens am Objekt zusammentreffen. Ein berechneter Accessor-Name bleibt Lücke. Erledigt. | `({get x(){return 5}}).x` antwortet wie im Stack-Backend. |
| G6q | 15.7.14: Eine Instruktion baut den Konstruktor samt seinem `prototype`, dessen Attribute keine gewöhnliche Funktion hat; jede Methode bekommt die Attribute aus 7.3.5, statisch am Konstruktor, sonst am Prototype. Der `[[Call]]` des Konstruktors wirft. `extends` und berechnete Namen bleiben Lücken. Erledigt. | `class C{m(){return 7}}; new C().m()` antwortet wie im Stack-Backend. |
| G6r | 14.3.3.3 über einen Wert ohne bekanntes Layout: jede Property wird zur Laufzeit gelesen, 7.2.1 vor dem ersten Lesen geprüft, und ein Initializer greift zur Laufzeit statt verloren zu gehen. Ein Pattern ohne Property und ein Rest-Element bleiben Lücken. Erledigt. | `function f(o){let {a=7}=o;return a}f({})` antwortet wie im Stack-Backend. |
| G6s | Ein Parameter, der ein Object-Pattern ist (10.2.11, 8.6.2): das Register des Aufrufs bekommt eine Bindung, die kein Identifier eines Scripts sein kann, und der Rumpf beginnt mit 14.3.3.3 darauf. Ein Array-Pattern bleibt Lücke (Iterator, 7.4.9). Erledigt. | `function f({a=3}){return a}; f({})` antwortet wie im Stack-Backend. |
| G6t | 8.6.2 für ein Array-Pattern: 7.4.2 öffnet den Iterator, jedes Element ist ein Schritt aus 7.4.6, `[[Done]]` steht in einem Register, und 7.4.9 schließt, was übrig blieb. Auch ein Parameter, der ein Array-Pattern ist. Ein Rest-Element bleibt Lücke. Erledigt. | `function f([a,b=2]){return a+b}; f([1])` antwortet wie im Stack-Backend. |
| G6u | Ein eingefangenes `var`, das ein Write erreicht, trägt den Typ, den das Lowering nicht benennen kann, statt den ganzen umgebenden Rumpf abzulehnen. Erledigt. | `var c=0;var f=function(){c=c+1};f();c` antwortet wie im Stack-Backend. |
| G6v | Ein Initializer aus 8.6.2/14.3.3.3, der ein Objekt macht: auf dem immer genommenen Pfad behält er sein Layout, auf dem bedingten wird es fallengelassen und die Antwort ist der Typ, den das Lowering nicht benennen kann. Ein geändertes Layout bleibt Ablehnung. Erledigt. | `function f(a={b:1}){return a.b}f()` antwortet wie im Stack-Backend. |
| G6w | 10.2.10 und 8.5.2: die Code-Unit trägt den Namen als eigene String-Konstante, und die Instruktion, die den Closure macht, definiert die Property mit den Attributen aus 10.2.10. Das Lowering setzt ihn dort, wo die Spezifikation ihn setzt. Erledigt. | `({m(){}}).m.name` ist `'m'`, wie 13.2.5.5 es sagt. |
| G6x | Das Arguments-Objekt einer strikten Funktion (10.4.4) ist ein Wert wie jeder andere: 10.2.4.1 als Intrinsic, das auf keinem Objekt steht, `callee` als dessen Accessor, dazu der Iterator aus 23.1.3.33. Der Iterator läuft jetzt auch über ein Array-like (23.1.5.2.1). Erledigt. | `'use strict';function f(){return arguments}f(1,2).length` antwortet wie im Stack-Backend. |
| G6y | Ein Property-Write braucht kein Layout mehr (13.15.2), und 10.4.2.4 setzt die eigene `length` eines Arrays und löscht, was darüber steht. Ein Index, den die Shape in 10.4.2.1 übernommen hat, bleibt dort Lücke. Erledigt. | `var a=[1,2,3];a.length=2;a[2]` antwortet wie im Stack-Backend. |
| G6z | 13.4.4.1 auf einem Namen, den das Global Environment Record bindet (9.1.1.4), und auf einer Property-Reference, die einmal ausgewertet wird. Die beiden Scope-Analysen laufen jetzt über ein Update einer Property. Erledigt. | `var o={n:0};o.n++;o.n` antwortet wie im Stack-Backend. |
| G7a | Ein `var`-Head aus 16.1.7 lebt im Global Environment Record: die Schleife hält Key oder Element in einem eigenen Register und schreibt die Bindung dorthin, wo 14.7.5.6 sie hat. Erledigt. | `for(var k in {a:1}){}` im Realm antwortet wie im Stack-Backend. |
| G7b | 13.10.2: `HasProperty` aus 7.3.11 über eigene Properties und die Prototype Chain, mit einer Lücke für einen Namen, den ein nicht gebautes Prototype besäße. Erledigt. | `'a' in {a:1}` antwortet wie im Stack-Backend. |
| G7c | 20.4 als Konstruktor, mit den dreizehn Symbolen aus Tabelle 1 (20.4.2). Ein eigenes Symbol zu machen bleibt Lücke. 7.1.19 behält ein Symbol als den Key, der es ist — die drei Instruktionen mit berechnetem Key schickten jeden durch `ToString`. Erledigt. | `({[Symbol.iterator](){}})[Symbol.iterator]` antwortet wie im Stack-Backend. |
| G7d | 22.2.4.1: das Pattern wird dort kompiliert, wo das Script kompiliert wird, und liegt in der Code-Unit neben ihren String-Konstanten; das Objekt nennt Unit und Index, wie ein Closure seine Funktion nennt. 22.2.7.2 läuft auf demselben Automaten wie im Stack-Backend. Konstruktor und die Accessors aus 22.2.6 bleiben Lücken. Erledigt. | `/(a)(b)/.exec('xabz')[1]` antwortet wie im Stack-Backend. |
| G7e | 10.1.9.1: die beiden Store-Instruktionen tragen die Strictness der Reference und fragen die Property, bevor sie schreiben; dazu tragen sie, ob definiert statt zugewiesen wird (13.2.5.5). `__proto__` bleibt abgelehnt. Erledigt. | `var f=function(){};f.length=9;f.length` antwortet wie im Stack-Backend. |
| G7f | 14.7.5: Der Head einer `for`-`in`- oder `for`-`of`-Schleife darf ein Binding Pattern sein. Die Schleife hält den Schritt in einem eigenen Register, der Rumpf bindet daraus die Namen aus 8.6.2, und ein lexikalischer Head gibt seine Register in der Reihenfolge zurück, die der Allocator will. Erledigt. | `for(const [a,b] of [[1,2]]){}` antwortet wie im Stack-Backend. |
| G7g | 25.5: die Arena aus dem Stack-Backend parst den Text, und weil sie in Postorder steht, wird jeder Wert nach allem gebaut, was er hält, und hält seinen eigenen Root, während der nächste alloziert wird. 25.5.2 schreibt den Text. Reviver, Replacer, Space und `toJSON` bleiben Lücken. Erledigt. | `JSON.parse(JSON.stringify({a:[1,2]})).a[1]` antwortet wie im Stack-Backend. |
| G7h | 23.1.3: Ein Element, das ein Accessor ist, laeuft seinen Getter dort, wo eine Data Property nur gelesen wird. Der Walk-State traegt ein `getter`-Flag und betritt den Getter mit demselben `Resume::Iteration` wie den Callback. 10.1.8.1 Schritt 3.b liest `undefined`, wenn kein Getter da ist. Erledigt. | `[1,2].map` ueber ein Accessor-Element antwortet wie im Stack-Backend. |
| G7i | 8.6.2: Ein BindingRestElement sammelt den Rest des Iterators in ein eigenes Array, dessen Indizes 13.2.5.5 definiert. Die Lowering emittiert diese Schleife; der Layout-Pfad, der die Laenge kennt, hatte sie schon. Erledigt. | `function f(v){let [a,...r]=v;return r.join()}f([1,2,3])` antwortet wie im Stack-Backend. |
| G7j | 13.15.5.5: Ein Array-Pattern einer Destructuring Assignment nimmt seine Elemente aus dem Iterator des Werts, also aus demselben Walk wie 8.6.2; die Assignment-Form wertet jede Ziel-Reference vor dem Schritt aus. Ein Object-Pattern liest nach 7.3.5, und ein Name ohne Binding des Scripts wird nach 9.1.1.4.5 auf dem Global Environment Record geschrieben. Erledigt. | `function f(v){var a,b;[a,b]=v;return a+b}f([1,2])` antwortet wie im Stack-Backend. |
| G7k | 22.1.3.23: `String.prototype.split` fuer einen Separator, der kein Object ist. Die Teile werden aus dem Text geschnitten, bevor etwas alloziert wird, damit kein String eines Teils ungerootet liegt, waehrend der naechste entsteht. Ein RegExp-Separator traegt das `@@split` aus 22.2.6.14, das dieses Engine noch nicht hat: benannte Luecke. Erledigt. | `'a,b,c'.split(',').join('|')` antwortet wie im Stack-Backend. |
| G7l | 22.1.3.14 und 22.1.3.20 ueber die Methoden aus 22.2.6: 22.2.6.8 antwortet ohne `g` wie 22.2.7.2 und laeuft mit `g` den ganzen Text ab Index null ab, wobei ein leerer Treffer nach 22.2.7.3 um eine Code Unit weiterrueckt; 22.2.6.12 sucht ab dem Anfang und laesst `lastIndex`, wie es war. Erledigt. | `'a1b2'.match(/[0-9]/g).join('|')` antwortet wie im Stack-Backend. |
| G7m | 22.2.6.14 splittet an jedem Sticky-Treffer und haengt die Captures jedes Treffers an; der Splitter unterscheidet sich vom Receiver nur durch `y`, also matcht der Walk sticky statt ein zweites RegExp zu bauen. Erledigt. | `'a1b'.split(/([0-9])/).join('|')` antwortet wie im Stack-Backend. |
| G7n | 7.1.1 fuer jedes Argument, das eine Klausel konvertiert: eine Tabelle nennt die Argumente in der Reihenfolge der Klausel, jedes mit seinem Hint, und der Native laeuft von vorn, bis keines mehr offen ist. 7.1.4 uebergibt den Hint `number`. 22.1.3 und 23.1.3 pruefen zuerst den `this`-Wert. Erledigt. | `'abcdef'.slice({valueOf(){return 1}},{valueOf(){return 3}})` antwortet wie im Stack-Backend. |
| G7o | 14.15: Der Catch Parameter wird nach 8.6.2 gebunden, ein Pattern deklariert also je Namen ein Binding und fuellt es aus dem geworfenen Wert. Da eine Exception an jeder Stelle des geschuetzten Bereichs auftreten kann, traegt ein Binding, das der Block schreibt, im Handler die Spitze des Verbands; ein Finally Block laeuft auf jedem Pfad und behaelt daher die strengere Regel. Erledigt. | `try{throw {a:1,b:2}}catch({a,b}){a+b}` antwortet wie im Stack-Backend. |
| G7p | 19.2.2 bis 19.2.5 als vier Value Properties des globalen Objekts. Die beiden Parser aus 19.2.4 und 19.2.5 wandern aus dem Stack-Backend nach `crates/jrs/src/number.rs`, das jetzt beide Backends rufen. 19.2.3 gibt `isNaN` einen formalen Parameter, den das Stack-Backend auf null laesst; die Engine folgt der Spezifikation. Erledigt. | `parseInt('0x1f')` antwortet wie im Stack-Backend. |
| G7q | 13.2.8.6: Jede Substitution eines Template Literals laeuft durch 7.1.17, also 7.1.1 mit Hint `string` und nicht dem, den `+` gibt. Eine eigene Instruktion konvertiert ein Register; ein Object erreicht 7.1.1, dessen Methode ins Register zurueckkommt, und die Instruktion laeuft erneut. Erledigt. | ``var o={toString(){return 'T'},valueOf(){return 9}};`v=${o}` `` antwortet wie im Stack-Backend. |
| G7r | Ein Store unter einem Namen, den das Objekt selbst besitzt, laeuft nach 10.1.9.1 auf dem Objekt und erreicht gar kein Prototype; nur `__proto__` aus B.2.2.1 und die `length` eines Arrays mit dem eigenen `[[DefineOwnProperty]]` aus 10.4.2.1 bleiben Luecken. Ein Object-Pattern ohne Property wird wie jedes andere gelowered, weil 14.3.3.3 ohnehin auf Coercible prueft. Erledigt. | `var f=function(o,k){o[k]='x'};var g=function(){};f(g,'name')` antwortet wie im Stack-Backend. |
| G7s | 10.4.4.7 bildet die Indizes des Arguments-Objekts einer Sloppy-Funktion auf ihre formalen Parameter ab, was diese Engine nicht baut; eine Funktion ohne formalen Parameter hat eine leere Abbildung, also geht ihr Objekt dorthin, wohin jeder andere Wert geht. Erledigt. | `(function(){return arguments})(1,2).length` antwortet wie im Stack-Backend. |
| G7t | 14.7.5.6 Schritt 7.g: Ein Head, der nichts deklariert, wertet sein Ziel als eigene Reference aus, nachdem der Schritt den Wert geliefert hat, und zwar je Iteration. Jeder Name, den das Ziel schreibt, traegt ab dem Head die Spitze des Verbands. Erledigt. | `var a;for(a of [1,2]){}a` antwortet wie im Stack-Backend. |
| G7u | 9.1.1.1.1: Eine lexikalische Deklaration, deren Initializer einen von ihr gebundenen, noch nicht initialisierten Namen liest, wird nicht gelowered; `let i = i++` antwortete NaN, wo die Sprache wirft. Ein Methodenaufruf auf einer Number oder einem Boolean loest auf dem Prototype auf, den 7.1.18 dem Wrapper gaebe, und 21.1.3.6 nimmt jede Radix. Erledigt. | `(255).toString(16)` antwortet wie im Stack-Backend, `let i=i++` bleibt beim Stack-Backend. |
| G7v | 20.4.1.1 macht ein Symbol, das kein anderer Wert ist: der Heap haelt die Description jedes von einem Script gemachten Symbols hinter den dreizehn aus Tabelle 1, und die Registry aus 20.4.2.2 bildet einen Key darauf ab. 20.4.3 antwortet ohne den Wrapper aus 7.1.18. 7.1.17 und 7.1.4 eines Symbols sind ein TypeError. Erledigt. | `Symbol('a').description` und `Symbol.for('k')===Symbol.for('k')` antworten wie im Stack-Backend. |
| G7w | 7.1.17 eines Objekts ist 7.1.1 mit Hint `string`. Ein Objekt, das weder ein `@@toPrimitive` noch ein `toString` des Scripts traegt, antwortet ohne Frame: `%Object.prototype%.toString` und die Wrapper-Prototypes lesen den internen Slot. 22.2.7.1 und 22.2.6.16 konvertieren ihren Text wie jedes andere Argument. Erledigt. | `new String('abc').split('b').join('|')` antwortet wie im Stack-Backend. |
| G7x | 15.7.14 und 13.2.5.5 definieren eine Methode oder einen Accessor unter dem Key, den 7.1.19 aus dem Wert macht; 10.2.10 benennt die Funktion danach. Zwei Instruktionen nehmen den Key aus einem Register. Eine Property, die ein Layout mit einem unbenennbaren Typ beantwortet, ist ein Callee, ueber den 7.3.14 zur Laufzeit entscheidet. Erledigt. | `var k='m';class C{[k](){return 1}};(new C()).m()` antwortet wie im Stack-Backend. |
| G7y | 23.1.3.12 und 23.1.3.13 sind der Walk aus 23.1.3 rueckwaerts. Sie und 23.1.3.9 und 23.1.3.10 lesen jeden Index mit 7.3.2, ein Loch erreicht den Callback also als `undefined`. 23.1.3.17 und 23.1.3.4 antworten den Array Iterator aus 23.1.5 ueber Index beziehungsweise Index und Element. Erledigt. | `[1,2,3].findLast(x=>x<3)` und `[7,8].entries()` antworten wie im Stack-Backend. |
| G7z | 20.2.3.1 und 28.1.1 rufen mit der Liste, die 7.3.18 aus einem Array-Like macht. Kein Frame des Callers haelt sie, also traegt der Call das Array, aus dem der Frame seine Parameter nimmt und aus dem 10.4.4 das Arguments-Objekt baut. Ein in Rust geschriebener Callee liest aus Registern des Callers: benannte Luecke. Erledigt. | `function f(a,b){return a+b};f.apply(null,[1,2])` antwortet wie im Stack-Backend. |
| G8a | 7.1.20 liest eine `length`, die ein Accessor ist, ueber ihren Getter, und der laeuft vor dem Walk aus 23.1.3. Der Walk nimmt seinen State also vor der Laenge und betritt diesen Getter wie den Callback; die Pruefungen aus 23.1.3 laufen auf dem Rueckweg. 10.4.2.2 Schritt 1 bleibt auf beiden Pfaden. Erledigt. | `Array.prototype.map.call({get length(){return 2},0:1,1:2},f)` antwortet wie im Stack-Backend. |
| G8b | 23.1.3.16, 23.1.3.17 und 23.1.3.20 lesen jeden Index wie jede andere Klausel und laufen deshalb durch denselben Walk: ein Element, das ein Accessor ist, laeuft seinen Getter. Sie vergleichen selbst und oeffnen keinen Frame dafuer; der State haelt das gesuchte Element dort, wo sonst der Callback steht. Erledigt. | `var a=[1,2];Object.defineProperty(a,0,{get(){return 9}});a.indexOf(9)` antwortet wie im Stack-Backend. |
| G8c | 10.4.2.4 als `[[DefineOwnProperty]]`: die `length` eines Arrays ist nie enumerable und nie configurable, und ihr `[[Writable]]` geht nur von wahr nach falsch, was das Array jetzt neben der Laenge traegt. Ein Deskriptor mit Wert verschiebt die Laenge und loescht jeden Index darueber. Erledigt. | `var a=[1,2,3];Object.defineProperty(a,'length',{value:1});a.length` antwortet wie im Stack-Backend. |
| G8d | 16.1.7 macht eine top-level lexikalische Deklaration zu einem Binding des Global Environment Record, das 8.6.2 Name fuer Name fuellt. Ein lexikalisches Binding wird initialisiert und nicht geschrieben, was es aus seiner Dead Zone holt. Erledigt. | `const [x]=[1];x` antwortet wie im Stack-Backend. |
| G8e | 20.1.2.22 und 28.1.14 laufen beide `OrdinarySetPrototypeOf` aus 10.1.2: derselbe Wert wird immer genommen, ein anderer nur solange das Objekt extensible ist und die Kette azyklisch bleibt. Erledigt. | `var o={};Object.setPrototypeOf(o,{x:1});o.x` antwortet wie im Stack-Backend. |
| G8f | 20.2.3.5 antwortet den Quelltext des Grammatik-Knotens, als der die Funktion geschrieben wurde; der Parser hielt ihn schon, die Unit traegt ihn jetzt als eigene String-Konstante. Zuvor erreichte ein Script `%Object.prototype%.toString` und las `[object Function]` -- eine falsche Antwort. Erledigt. | `function f(a){return a};f.toString()` antwortet wie im Stack-Backend. |
| G8g | 14.2.3 verlaesst den Block mit dem Binding, also ist ein Object-Binding keines anders als ein primitives: das Register ist ein Slot des Frames, den der Collector ohnehin scannt, und der Name ist mit dem Block weg. Erledigt. | `{let o={a:2};o.a}` antwortet wie im Stack-Backend. |
| G8h | 14.3.3.3 sammelt das Rest-Element mit `CopyDataProperties` aus 7.3.25. Fuer einen Wert, den die Lowering nicht benennen konnte, kennt nur die Laufzeit die eigenen enumerablen Keys: eine eigene Instruktion kopiert sie und laesst die Namen aus, die das Pattern schon genommen hat. Erledigt. | `function f(o){var {a,...r}=o;return r.b}f({a:1,b:2})` antwortet wie im Stack-Backend. |
| G8i | 7.4.9 schliesst einen Iterator, den die Schleife vor seinem Ende verlassen hat; die Schleife emittiert den Close dort, wo ein `break` landet. Der normale Ausgang hat das Ende erreicht und schliesst nichts, `continue` geht zum Head. Ein `return` verlaesst den Frame an dem Close vorbei und bleibt eine benannte Luecke. Erledigt. | Ein `break` aus `for(x of it)` ruft `it.return()` wie im Stack-Backend. |
| G8j | 20.1.3.7 beantwortet das Objekt, das `ToObject` aus dem `this`-Wert macht. Ohne diese Methode fand 7.1.1 auf der Prototype Chain eines gewoehnlichen Objekts kein `valueOf`, also war jedes `ToPrimitive` eines Objekts ohne eigene Methode eine Luecke. 20.1.3.5 bleibt eine: das Stack-Backend hat sie nicht, und beide Pfade muessen gleich antworten. Erledigt. | `''+{}` ergibt `[object Object]` wie im Stack-Backend. |
| G8k | Vier Antworten, die der Full-Suite-Diff gegen das Stack-Backend benannt hat: 23.1.3.2 Schritt 3 macht den Receiver zum ersten Item, das 23.1.3.2.1 nur als Array spreizt; 20.5.6.2 gibt einem nativen Error-Konstruktor `%Error%` als [[Prototype]]; 13.10.2 Schritt 5 wirft fuer eine rechte Seite, die nicht aufrufbar ist; 20.2.3 macht `%Function.prototype%` selbst zu einer eingebauten Funktion mit `length` 0 und `name` "". Erledigt. | `typeof Function.prototype` ist `function`, und `({}).concat(1)[0]` ist das Objekt. |
| G8l | Drei weitere Antworten: 10.4.3.1 gibt einem String Exotic Object `length` und jeden Index seiner `[[StringData]]`, die keine Shape haelt; 7.3.2 liest einen Index der Prototype Chain wie einen Namen, nicht nur den Elements-Store des Receivers; 23.1.3.24 Schritt 6 wirft, wenn der Walk kein Element fand und keinen Startwert bekam. Erledigt. | `Array.prototype.map.call('abc',f)` laeuft ueber drei Elemente. |
| G7h | Die Konstruktoren, an denen die Varianten jetzt stehen: `%Number%` (21.1), `%Boolean%` (20.3), dazu `%Reflect%` (28.1) und `%Symbol%` (20.4). | Die Varianten, die heute an einem dieser Namen stehen, erreichen ihre Assertions. |
| G5 | Ein Object des Engine-Cores kann die Grenze zum Embedding überqueren, wo ein Aufrufer den Wert wirklich liest. Das ist das gemeinsame Objektmodell aus M3 und M4, keine Lücke des Lowerings. | `Realm::evaluate` gibt ein Object zurück, statt es als Lücke zu melden. |

G0 steht vorn, weil G2 und G3 ohne ihn nicht fertig werden können. Das Lowering
typisiert jeden Wert statisch und lehnt ab, was es nicht typisieren kann. Der
Typ eines globalen Namens steht nie statisch fest, also ist jeder globale Wert
möglicherweise ein Object, und `a + b` über einem Object verlangt ToPrimitive,
das ein `valueOf` des Benutzers aufrufen kann. Eine Operation des Engine-Cores
kann heute keinen Frame öffnen: `primitive_binary` erreicht `primitive_number`,
das für ein Object mit einem TypeError endet. Solange das so ist, kann ein
globaler Name gelesen, aber mit nichts verrechnet werden.

Zwei falsche Antworten, die auf diesem Weg gefunden wurden, zeigen, wofür die
Regel "Lücke statt Antwort" da ist: ein Fehltreffer auf einer noch nicht
gebauten Prototype antwortete `undefined`, und `arguments` wurde auf dem Global
Environment Record aufgelöst statt nach 10.4.4 in der Funktion. Beide waren vom
Lowering verdeckt und wurden erst sichtbar, als es weiter reichte. Jede
Erweiterung des Lowerings deckt deshalb Stellen auf, die vorher unerreichbar
waren; der Differential-Fuzzer und der variantengenaue Test262-Vergleich sind
die Werkzeuge, die sie finden.

Seit G5i lädt der Harness im Engine-Core, also messen die `--engine`-Läufe ihn
wirklich. Zahlen davor sind Zahlen des Stack-Backends.

Aus G5a folgt eine Grenze, die mit `eval` fällig wird: der Realm hält jede Unit,
die er ausgeführt hat, weil ein Funktionsobjekt einer früheren Unit aufrufbar
bleibt. Solange nur Scripts Units erzeugen, ist ihre Zahl durch die Zahl der
Scripts begrenzt. Sobald `eval` übersetzt wird, erzeugt jeder Aufruf eine Unit,
und der Realm braucht ein Kriterium, wann eine Unit nicht mehr erreichbar ist —
das ist eine Frage des Kollektors, nicht des Lowerings.

## 18. Erster umsetzbarer Arbeitsauftrag nach Freigabe

Der nächste Architektur-Arbeitsschritt sollte keine neue große Sprachfunktion sein, sondern ein begrenzter Pilot:

1. Ein strukturiertes Inventar der öffentlichen APIs, Heap-Roots, Realm-Zustände und Native-Reentry-Stellen erzeugen.
2. Eine gemeinsame Outcome-/Completion-Grenze definieren und an einem vorhandenen Builtin-Familienpfad testen.
3. `Runtime::run` und persistente Script-Ausführung auf dieselbe Global-Instantiation ausrichten; bisherige Unterschiede durch Regressiontests festhalten.
4. Einen Constructor vollständig auf echte Function-Objekte statt Property-Schattenstorage migrieren, einschließlich Mutation, Alias, Species und Realm-Zuordnung.
5. Vorher-/Nachher-Konformität, GC-Stress, Allocation-Zahlen und Performance vergleichen; erst dann den Vertrag auf weitere Constructoren ausrollen.

Der Pilot ist fertig, wenn der gemeinsame Vertrag funktioniert und die bestehenden Tests ohne verdeckte Ausnahmen weiterlaufen. Er ist nicht gleichbedeutend mit dem Abschluss des gesamten Goals.

Schritt 1 ist erledigt: [jrs-inventory.md](jrs-inventory.md) hält den Ausgangsstand fest. Der Befund dieses Inventars ist, dass der vorhandene registerbasierte Pfad unter `crates/jrs/src/engine/` eine zweite Sprachsemantik mit eigenem Objektmodell, eigenem Heap und eigenen Intrinsics ist und keine Test262-Datei ausführt.

Die Entscheidung dazu lautet: Dieser Pfad ist das Ziel, keine spätere Optimierung. Die Doppelung wird nicht eingefroren, sondern nach Abschnitt 17.1 aufgelöst, indem die Semantik des Stack-Backends dorthin überführt und der alte Pfad entfernt wird. Die erste zu schließende Lücke ist der Einstieg: `Realm::evaluate` kompiliert mit `realm = true` und lehnt in diesem Modus jede Deklaration, jedes `var`, jede Funktion und jeden lexikalischen Block ab, weshalb der Test262-Runner den Engine-Core nie erreicht. Bis diese Lücke geschlossen ist, misst jede Test262-Zahl in [crates/jrs/README.md](../crates/jrs/README.md) ausschließlich das Stack-Backend.

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
