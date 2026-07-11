// Smoke test for the *shipped* website wasm artifact, SwiftUI entry points.
//
// The production playground (`src/components/FullPlayground.astro`) drives the
// live SwiftUI preview through `swiftUICompile` / `swiftUIDispatch` exported by
// `public/wasm/tswift_wasm.js`. Native `cargo test` can't catch wasm-only
// panics, and a stale `runSwift`-only wasm would silently break the preview, so
// this loads the real `.wasm` the site serves and drives it through Node:
// every SwiftUI preset compiles + renders a UIIR tree, and a counter tap
// mutates @State and returns a minimal in-place patch.
//
// Run with: npm test

import { readFileSync } from 'node:fs';
import process from 'node:process';

const wasmDir = new URL('../public/wasm/', import.meta.url);
const { initSync, swiftUICompile, swiftUIDispatch, swiftDiagnostics } = await import(
  new URL('tswift_wasm.js', wasmDir)
);
initSync({ module: readFileSync(new URL('tswift_wasm_bg.wasm', wasmDir)) });

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (err) {
    failures += 1;
    console.error(`  FAIL ${name}\n       ${err.message}`);
  }
}
function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

// 1. The SwiftUI host exports exist (a stale runSwift-only wasm would break the
//    preview silently).
check('wasm exports the SwiftUI host entry points', () => {
  assert(typeof swiftUICompile === 'function', 'swiftUICompile missing');
  assert(typeof swiftUIDispatch === 'function', 'swiftUIDispatch missing');
});

// 2. A counter compiles to a full tree, and a button tap returns a minimal
//    in-place patch stream (the wire format `<swiftui-canvas>` applies).
check('counter renders and a tap returns a setText patch', () => {
  const src = [
    'struct CounterView: View {',
    '  @State private var count = 0',
    '  var body: some View {',
    '    VStack {',
    '      Text("\\(count)")',
    '      Button("Increment") { count += 1 }',
    '    }',
    '  }',
    '}',
  ].join('\n');
  const r = JSON.parse(swiftUICompile(src));
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(r.root === 'CounterView', `bad root: ${r.root}`);
  assert(r.tree.kind === 'VStack', `bad root kind: ${r.tree.kind}`);
  const after = JSON.parse(swiftUIDispatch('0.1', 'tap', ''));
  assert(after.ok === true, `dispatch failed: ${JSON.stringify(after)}`);
  assert(Array.isArray(after.patches), `expected a patches array: ${JSON.stringify(after)}`);
  const setText = after.patches.find((p) => p.op === 'setText' && p.id === '0.0');
  assert(setText && setText.text === '1', `tap did not emit setText=1: ${JSON.stringify(after.patches)}`);
});

// 3. A program with no View is a structured compile error, not a wasm trap —
//    the playground falls back to the text `runSwift` path in this case.
check('missing View returns a structured error (text-mode fallback)', () => {
  const r = JSON.parse(swiftUICompile('let x = 1'));
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  assert(r.root == null, `expected root=null, got ${r.root}`);
});

// 4. The text-mode (runSwift) playground presets.
const { runSwift } = await import(new URL('tswift_wasm.js', wasmDir));
// Mirrors every preset in FullPlayground.astro — keep these in sync.
// Text-mode presets run via runSwift; SwiftUI presets are covered below via swiftUICompile.
const TEXT_PRESETS = {
  'Hello World': [
    'let language = "Swift"',
    'let version = 6',
    'print("Hello from \\(language) \\(version)! 👋")',
    'let π = 3.14159',
    'let radius = 5.0',
    'let area = π * radius * radius',
    'print("Circle area (r=\\(radius)): \\(area)")',
    'let score = 87',
    'let grade = score >= 90 ? "A" : score >= 80 ? "B" : score >= 70 ? "C" : "F"',
    'print("Score \\(score) → Grade \\(grade)")',
  ].join('\n'),
  'Fibonacci': [
    'func fib(_ n: Int) -> Int {',
    '    if n < 2 { return n }',
    '    return fib(n - 1) + fib(n - 2)',
    '}',
    'for i in 0...10 { print("fib(\\(i)) = \\(fib(i))") }',
    'func fibFast(_ n: Int) -> Int {',
    '    var a = 0, b = 1',
    '    for _ in 0..<n { (a, b) = (b, a + b) }',
    '    return a',
    '}',
    'print("fibFast(20) = \\(fibFast(20))")',
  ].join('\n'),
  'Closures & HOF': [
    'let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]',
    'let doubled = numbers.map { $0 * 2 }',
    'print("doubled: \\(doubled)")',
    'let evens = numbers.filter { $0 % 2 == 0 }',
    'print("evens: \\(evens)")',
    'let sum = numbers.reduce(0, +)',
    'print("sum: \\(sum)")',
    'func makeMultiplier(_ factor: Int) -> (Int) -> Int { { $0 * factor } }',
    'let triple = makeMultiplier(3)',
    'print("triple(7) = \\(triple(7))")',
    'let words = ["swift", "is", "fast"]',
    'print(words.map { $0.uppercased() }.joined(separator: " "))',
  ].join('\n'),
  'Structs': [
    'struct Point {',
    '    var x: Double',
    '    var y: Double',
    '    var magnitude: Double { (x*x + y*y).squareRoot() }',
    '    mutating func translate(dx: Double, dy: Double) { x += dx; y += dy }',
    '    func scaled(by factor: Double) -> Point { Point(x: x * factor, y: y * factor) }',
    '}',
    'var p = Point(x: 3, y: 4)',
    'print("magnitude: \\(p.magnitude)")',
    'var q = p',
    'q.translate(dx: 10, dy: 0)',
    'print("p.x=\\(p.x), q.x=\\(q.x)")',
    'let big = p.scaled(by: 2)',
    'print("scaled: (\\(big.x), \\(big.y))")',
  ].join('\n'),
  'Enums': [
    'enum Direction: CaseIterable {',
    '    case north, south, east, west',
    '    var opposite: Direction {',
    '        switch self {',
    '        case .north: return .south; case .south: return .north',
    '        case .east:  return .west;  case .west:  return .east',
    '        }',
    '    }',
    '}',
    'print(Direction.allCases)',
    'print(Direction.north.opposite)',
    'enum Planet: Int { case mercury = 1, venus, earth, mars }',
    'print(Planet.earth.rawValue)',
  ].join('\n'),
  'Optionals': [
    'let values: [Int?] = [1, nil, 3, nil, 5]',
    'for v in values {',
    '    if let x = v { print("got \\(x)") }',
    '    else { print("nil") }',
    '}',
    'func divide(_ a: Int, _ b: Int) -> Int? {',
    '    guard b != 0 else { return nil }',
    '    return a / b',
    '}',
    'print(divide(10, 2) ?? -1)',
    'print(divide(5, 0) ?? -1)',
  ].join('\n'),
  'Classes': [
    'class Animal {',
    '    let name: String',
    '    init(_ name: String) { self.name = name }',
    '    func speak() -> String { "..." }',
    '}',
    'class Dog: Animal { override func speak() -> String { "Woof!" } }',
    'class Cat: Animal { override func speak() -> String { "Meow!" } }',
    'let animals: [Animal] = [Dog("Rex"), Cat("Whiskers")]',
    'for a in animals { print("\\(a.name): \\(a.speak())") }',
    'class Counter { var count = 0; func increment() { count += 1 } }',
    'let c1 = Counter(); let c2 = c1',
    'c1.increment(); c1.increment()',
    'print("c1=\\(c1.count), c2=\\(c2.count)")',
  ].join('\n'),
  'Protocols': [
    'protocol Scorable { var score: Int { get }; func grade() -> String }',
    'extension Scorable {',
    '    func grade() -> String {',
    '        switch score { case 90...: return "A"; case 80..<90: return "B"; default: return "C" }',
    '    }',
    '}',
    'struct Student: Scorable { let name: String; let score: Int }',
    'let s = Student(name: "Ada", score: 95)',
    'print("\\(s.name): \\(s.grade())")',
  ].join('\n'),
  'Generics / popLast': [
    'func maxOf<T: Comparable>(_ a: T, _ b: T) -> T { a > b ? a : b }',
    'print(maxOf(3, 7))',
    'print(maxOf("apple", "banana"))',
    'struct Stack<Element> {',
    '    private var items: [Element] = []',
    '    mutating func push(_ item: Element) { items.append(item) }',
    '    mutating func pop() -> Element? { items.popLast() }',
    '    var top: Element? { items.last }',
    '}',
    'var s = Stack<Int>()',
    's.push(1); s.push(2); s.push(3)',
    'print(s.top!)',
    'while let x = s.pop() { print(x) }',
  ].join('\n'),
  'Error Handling': [
    'enum E: Error { case empty; case tooShort(Int) }',
    'func validate(_ s: String) throws -> String {',
    '    guard !s.isEmpty else { throw E.empty }',
    '    guard s.count >= 4 else { throw E.tooShort(s.count) }',
    '    return "ok"',
    '}',
    'let inputs = ["", "hi", "hello"]',
    'for p in inputs {',
    '    do { print(try validate(p)) }',
    '    catch E.empty { print("empty") }',
    '    catch E.tooShort(let n) { print("too short: \\(n)") }',
    '}',
  ].join('\n'),
  'Collections': [
    'var fruits = ["apple", "banana", "cherry"]',
    'fruits.append("date")',
    'print(fruits.sorted())',
    'print(fruits.filter { $0.count > 5 })',
    'var scores: [String: Int] = ["Alice": 95, "Bob": 72]',
    'scores["Eve"] = 88',
    'print(scores.values.reduce(0, +) / scores.count)',
    'let set1: Set = [1, 2, 3, 4]',
    'let set2: Set = [3, 4, 5, 6]',
    'print(set1.intersection(set2).sorted())',
  ].join('\n'),
};
for (const [label, code] of Object.entries(TEXT_PRESETS)) {
  check(`text preset "${label}" runs ok`, () => {
    const r = JSON.parse(runSwift(code));
    assert(r.run?.ok === true,
      `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  });
}

// 4b. Swift Concurrency through the shipped binary. A stale wasm (built before
//     PR #239 landed AsyncStream/TaskGroup/async-let) compiles fine but traps
//     at runtime with `unknown function: AsyncStream` — this asserts the served
//     binary actually carries the concurrency runtime, not just the source tree.
check('AsyncStream program runs and produces the expected output', () => {
  const src = [
    'func run() async {',
    '    let stream = AsyncStream(Int.self) { cont in',
    '        for i in 1...3 { cont.yield(i) }',
    '        cont.finish()',
    '    }',
    '    var sum = 0',
    '    for await x in stream { sum += x }',
    '    print("sum \\(sum)")',
    '    let deferred = AsyncStream(Int.self) { cont in',
    '        Task {',
    '            cont.yield(10)',
    '            cont.yield(20)',
    '            cont.finish()',
    '        }',
    '    }',
    '    var collected: [Int] = []',
    '    for await x in deferred { collected.append(x) }',
    '    print("collected \\(collected)")',
    '}',
    'run()',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'sum 6\ncollected [10, 20]\n',
    `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

// 5. The SwiftUI playground presets (mirrored from FullPlayground.astro) all
//    compile + render a tree.
const SWIFTUI_PRESETS = {
  Toggle: [
    'struct GreetingView: View {',
    '  @State private var isOn = true',
    '  var body: some View {',
    '    VStack(spacing: 16) {',
    '      Toggle("Show greeting", isOn: $isOn)',
    '      if isOn { Text("Hello!") }',
    '    }',
    '  }',
    '}',
  ].join('\n'),
  List: [
    'struct FruitList: View {',
    '  let fruits = ["Apple", "Banana", "Cherry"]',
    '  var body: some View {',
    '    List {',
    '      ForEach(fruits, id: \\.self) { fruit in',
    '        HStack { Text(fruit); Spacer(); Text("🍎") }',
    '      }',
    '    }',
    '  }',
    '}',
  ].join('\n'),
  Profile: [
    'struct ProfileCard: View {',
    '  var body: some View {',
    '    VStack(spacing: 12) {',
    '      Text("🦜").font(.largeTitle)',
    '      Text("Unlucky Parrot").font(.title).fontWeight(.bold)',
    '      Text("SwiftUI on tswift").foregroundColor(.secondary)',
    '    }.padding()',
    '  }',
    '}',
  ].join('\n'),
};
for (const [label, code] of Object.entries(SWIFTUI_PRESETS)) {
  check(`preset "${label}" renders`, () => {
    const r = JSON.parse(swiftUICompile(code));
    assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r.error || r)}`);
    assert(r.tree && typeof r.tree.kind === 'string', 'expected a UIIR tree with a root kind');
  });
}

// 5. The editor's live error-feedback channel: `swiftDiagnostics` lints without
//    running and reports structured positions/severities (CodeMirror's linter
//    maps these to inline squiggles).
check('swiftDiagnostics is exported and lints clean source', () => {
  assert(typeof swiftDiagnostics === 'function', 'swiftDiagnostics missing');
  const r = JSON.parse(swiftDiagnostics('let x = 1\nprint(x)'));
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(Array.isArray(r.diagnostics) && r.diagnostics.length === 0,
    `expected no diagnostics, got ${JSON.stringify(r.diagnostics)}`);
});

check('swiftDiagnostics reports an error with line/col/severity', () => {
  const r = JSON.parse(swiftDiagnostics('#error("boom")'));
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  const d = r.diagnostics[0];
  assert(d && d.severity === 'error', `expected an error diagnostic: ${JSON.stringify(r)}`);
  assert(typeof d.line === 'number' && typeof d.col === 'number',
    `expected numeric line/col: ${JSON.stringify(d)}`);
  assert(d.message.includes('boom'), `expected message to carry 'boom': ${JSON.stringify(d)}`);
});

// 6. `tswift.defaults.*` / `tswift.fs.*` host services (Slice 4) — the web
//    degraded tier from `src/lib/tswift-host-services.js`, backed here by a
//    minimal in-memory `localStorage` shim (Node has none). Exercises real
//    Swift source through `UserDefaults.standard` and `FileManager.default`,
//    matching the wire contract `crates/tswift-foundation` implements.
if (typeof globalThis.localStorage === 'undefined') {
  const store = new Map();
  globalThis.localStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
    get length() { return store.size; },
    key: (i) => [...store.keys()][i] ?? null,
  };
}
const { installTSwiftHostServices, __resetTSwiftHostServicesForTests, tswiftHostServiceCall } = await import(
  new URL('../src/lib/tswift-host-services.js', import.meta.url)
);

installTSwiftHostServices();

check('tswiftHostServices declares defaults + fs namespaces', () => {
  assert(
    Array.isArray(globalThis.tswiftHostServices) &&
      globalThis.tswiftHostServices.includes('tswift.defaults') &&
      globalThis.tswiftHostServices.includes('tswift.fs'),
    `bad tswiftHostServices: ${JSON.stringify(globalThis.tswiftHostServices)}`,
  );
  assert(typeof globalThis.tswiftHost === 'function', 'tswiftHost hook missing');
});

check('UserDefaults round-trips through localStorage', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let d = UserDefaults.standard',
    'd.set("web", forKey: "language")',
    'd.set(6, forKey: "version")',
    'print(d.string(forKey: "language") ?? "nil")',
    'print(d.integer(forKey: "version"))',
    'd.removeObject(forKey: "language")',
    'print(d.string(forKey: "language") ?? "nil")',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'web\n6\nnil\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('FileManager writes and reads a file through the virtual fs', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let fm = FileManager.default',
    'let path = "/tmp/tswift-playground/greeting.txt"',
    // The virtual fs (like the CLI\'s real one) requires the parent
    // directory to already exist before a write — no implicit `mkdir -p`.
    'try fm.createDirectory(atPath: "/tmp/tswift-playground", withIntermediateDirectories: true)',
    'try "hello playground".write(toFile: path, atomically: true, encoding: .utf8)',
    'print(fm.fileExists(atPath: path))',
    'let text = try String(contentsOfFile: path)',
    'print(text)',
    'try fm.removeItem(atPath: path)',
    'print(fm.fileExists(atPath: path))',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'true\nhello playground\nfalse\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('FileManager write fails without an existing parent directory', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let path = "/tmp/tswift-no-such-dir/greeting.txt"',
    'do {',
    '    try "hi".write(toFile: path, atomically: true, encoding: .utf8)',
    '    print("unexpected success")',
    '} catch {',
    '    print("caught")',
    '}',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'caught\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('FileManager write fails over an existing directory', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let fm = FileManager.default',
    'try fm.createDirectory(atPath: "/tmp/tswift-dir-target", withIntermediateDirectories: true)',
    'do {',
    '    try "hi".write(toFile: "/tmp/tswift-dir-target", atomically: true, encoding: .utf8)',
    '    print("unexpected success")',
    '} catch {',
    '    print("caught")',
    '}',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'caught\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('FileManager createFile with non-base64 contents is rejected, not stored as arbitrary text', () => {
  __resetTSwiftHostServicesForTests();
  const okResult = JSON.parse(
    tswiftHostServiceCall('tswift.fs.write', JSON.stringify(['/tmp/tswift-b64-test.txt', 'not-valid-base64!!', false])),
  );
  assert(okResult === false, `expected invalid base64 content to be rejected, got ${JSON.stringify(okResult)}`);
  const existsResult = JSON.parse(
    tswiftHostServiceCall('tswift.fs.exists', JSON.stringify(['/tmp/tswift-b64-test.txt'])),
  );
  assert(existsResult === false, 'a rejected write must not create the file');
});

check('FileManager \'..\' traversal resolves to the same virtual entry', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let fm = FileManager.default',
    'try fm.createDirectory(atPath: "/tmp/tswift-dotdot", withIntermediateDirectories: true)',
    'try "payload".write(toFile: "/tmp/tswift-dotdot/a.txt", atomically: true, encoding: .utf8)',
    'let text = try String(contentsOfFile: "/tmp/tswift-dotdot/sub/../a.txt")',
    'print(text)',
    'print(fm.fileExists(atPath: "/tmp//tswift-dotdot/./a.txt"))',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'payload\ntrue\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('listing the root directory works', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let fm = FileManager.default',
    'try fm.createDirectory(atPath: "/tswift-root-list-test", withIntermediateDirectories: true)',
    'let names = try fm.contentsOfDirectory(atPath: "/")',
    'print(names.contains("tswift-root-list-test"))',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'true\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('FileManager surfaces a thrown host error as a catchable Swift error', () => {
  __resetTSwiftHostServicesForTests();
  const src = [
    'import Foundation',
    'let fm = FileManager.default',
    'do {',
    '    try fm.removeItem(atPath: "/tmp/tswift-playground/does-not-exist.txt")',
    '    print("unexpected success")',
    '} catch {',
    '    print("caught")',
    '}',
  ].join('\n');
  const r = JSON.parse(runSwift(src));
  assert(r.run?.ok === true,
    `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  assert(r.run.stdout === 'caught\n', `unexpected stdout: ${JSON.stringify(r.run.stdout)}`);
});

check('mkdir with intermediates fails, not-a-directory-style, through an existing file component', () => {
  __resetTSwiftHostServicesForTests();
  tswiftHostServiceCall('tswift.fs.mkdir', JSON.stringify(['/tmp', true]));
  const write = JSON.parse(
    tswiftHostServiceCall('tswift.fs.write', JSON.stringify(['/tmp/tswift-mkdir-blocker', 'aGk=', false])),
  );
  assert(write === true, 'setup write must succeed');

  const mkdirReply = JSON.parse(
    tswiftHostServiceCall(
      'tswift.fs.mkdir',
      JSON.stringify(['/tmp/tswift-mkdir-blocker/sub/deeper', true]),
    ),
  );
  assert(
    typeof mkdirReply === 'object' && typeof mkdirReply.$thrown === 'string' &&
      mkdirReply.$thrown.includes('not a directory'),
    `expected a "not a directory" thrown error, got ${JSON.stringify(mkdirReply)}`,
  );

  // Nothing must have been created underneath the file component.
  const subExists = JSON.parse(
    tswiftHostServiceCall('tswift.fs.exists', JSON.stringify(['/tmp/tswift-mkdir-blocker/sub'])),
  );
  assert(subExists === false, `mkdir must not create descendants beneath a file, got exists=${subExists}`);
});

check('copy/move refuse a destination with a missing or non-directory parent', () => {
  __resetTSwiftHostServicesForTests();
  tswiftHostServiceCall('tswift.fs.mkdir', JSON.stringify(['/tmp', true]));
  const write = JSON.parse(
    tswiftHostServiceCall('tswift.fs.write', JSON.stringify(['/tmp/tswift-cm-src.txt', 'aGk=', false])),
  );
  assert(write === true, 'setup write must succeed');
  const parentFileWrite = JSON.parse(
    tswiftHostServiceCall('tswift.fs.write', JSON.stringify(['/tmp/tswift-cm-parent-is-file', 'aGk=', false])),
  );
  assert(parentFileWrite === true, 'setup write must succeed');

  const copyMissingParent = JSON.parse(
    tswiftHostServiceCall(
      'tswift.fs.copy',
      JSON.stringify(['/tmp/tswift-cm-src.txt', '/tmp/tswift-cm-no-such-dir/dst.txt']),
    ),
  );
  assert(
    typeof copyMissingParent === 'object' &&
      copyMissingParent.$thrown &&
      copyMissingParent.$thrown.includes('no such file or directory'),
    `expected a "no such file or directory" thrown error, got ${JSON.stringify(copyMissingParent)}`,
  );

  const copyParentIsFile = JSON.parse(
    tswiftHostServiceCall(
      'tswift.fs.copy',
      JSON.stringify(['/tmp/tswift-cm-src.txt', '/tmp/tswift-cm-parent-is-file/dst.txt']),
    ),
  );
  assert(
    typeof copyParentIsFile === 'object' &&
      copyParentIsFile.$thrown &&
      copyParentIsFile.$thrown.includes('not a directory'),
    `expected a "not a directory" thrown error, got ${JSON.stringify(copyParentIsFile)}`,
  );

  const moveMissingParent = JSON.parse(
    tswiftHostServiceCall(
      'tswift.fs.move',
      JSON.stringify(['/tmp/tswift-cm-src.txt', '/tmp/tswift-cm-no-such-dir/dst.txt']),
    ),
  );
  assert(
    typeof moveMissingParent === 'object' &&
      moveMissingParent.$thrown &&
      moveMissingParent.$thrown.includes('no such file or directory'),
    `expected a "no such file or directory" thrown error, got ${JSON.stringify(moveMissingParent)}`,
  );

  const moveParentIsFile = JSON.parse(
    tswiftHostServiceCall(
      'tswift.fs.move',
      JSON.stringify(['/tmp/tswift-cm-src.txt', '/tmp/tswift-cm-parent-is-file/dst.txt']),
    ),
  );
  assert(
    typeof moveParentIsFile === 'object' &&
      moveParentIsFile.$thrown &&
      moveParentIsFile.$thrown.includes('not a directory'),
    `expected a "not a directory" thrown error, got ${JSON.stringify(moveParentIsFile)}`,
  );

  // The rejected copy/move must not have removed the source.
  const srcStillExists = JSON.parse(
    tswiftHostServiceCall('tswift.fs.exists', JSON.stringify(['/tmp/tswift-cm-src.txt'])),
  );
  assert(srcStillExists === true, 'a rejected copy/move must leave the source untouched');
});

if (failures > 0) {
  console.error(`\n${failures} website swiftui wasm smoke check(s) failed`);
  process.exit(1);
}
console.log('\nall website swiftui wasm smoke checks passed');
