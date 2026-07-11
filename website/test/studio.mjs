// Unit tests for the Web Studio pure-logic modules (project/file store, module
// entry selection, symbol outline + quick-open). No DOM, no wasm — these are
// plain data transforms, tested the same lightweight way as the other
// website/test scripts.

import {
  createProject,
  addFile,
  renameFile,
  deleteFile,
  updateSource,
  validatePath,
  serialize,
  deserialize,
  sortFiles,
} from '../src/lib/studio/project.js';
import {
  moduleFiles,
  entryFile,
  isSwiftUIProject,
  declaresView,
} from '../src/lib/studio/module.js';
import {
  buildOutline,
  quickOpenIndex,
  filterEntries,
  fuzzyScore,
} from '../src/lib/studio/outline.js';
import { SAMPLES, sampleById } from '../src/lib/studio/samples.js';

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (err) {
    failures += 1;
    console.error(`  FAIL ${name}\n       ${err.stack || err.message}`);
  }
}
function assert(cond, msg) {
  if (!cond) throw new Error(msg || 'assertion failed');
}
function eq(a, b, msg) {
  assert(JSON.stringify(a) === JSON.stringify(b), `${msg || 'not equal'}: ${JSON.stringify(a)} !== ${JSON.stringify(b)}`);
}

console.log('project / file store');

check('createProject selects the first file', () => {
  const p = createProject('Demo', [{ path: 'main.swift', source: 'print(1)' }]);
  eq(p.activePath, 'main.swift');
  eq(p.files.length, 1);
});

check('addFile rejects duplicates and non-.swift names', () => {
  const p = createProject('Demo', [{ path: 'main.swift', source: '' }]);
  assert(validatePath(p, 'main.swift'), 'duplicate should be rejected');
  assert(validatePath(p, 'notes.txt'), 'non-swift should be rejected');
  assert(validatePath(p, ''), 'empty should be rejected');
  assert(validatePath(p, 'a//b.swift'), 'double slash rejected');
  assert(validatePath(p, 'Sources/Package.swift'), 'manifest must be at root');
  assert(validatePath(p, 'Sources/App.swift') === null, 'nested swift is fine');
});

check('addFile appends and makes the new file active', () => {
  let p = createProject('Demo', [{ path: 'main.swift', source: '' }]);
  p = addFile(p, 'Helper.swift', 'struct H {}');
  eq(p.activePath, 'Helper.swift');
  eq(p.files.map((f) => f.path).sort(), ['Helper.swift', 'main.swift']);
});

check('renameFile updates path + activePath and rejects clashes', () => {
  let p = createProject('Demo', [
    { path: 'main.swift', source: '' },
    { path: 'A.swift', source: '' },
  ]);
  p = renameFile(p, 'A.swift', 'B.swift');
  assert(p.files.some((f) => f.path === 'B.swift'));
  assert(!p.files.some((f) => f.path === 'A.swift'));
  let threw = false;
  try {
    renameFile(p, 'B.swift', 'main.swift');
  } catch {
    threw = true;
  }
  assert(threw, 'renaming onto an existing name should throw');
});

check('deleteFile refuses to empty the project and repoints active', () => {
  let p = createProject('Demo', [
    { path: 'main.swift', source: '' },
    { path: 'A.swift', source: '' },
  ]);
  p = deleteFile(p, 'main.swift');
  eq(p.files.length, 1);
  eq(p.activePath, 'A.swift');
  let threw = false;
  try {
    deleteFile(p, 'A.swift');
  } catch {
    threw = true;
  }
  assert(threw, 'deleting the last file should throw');
});

check('updateSource is immutable and path-scoped', () => {
  const p = createProject('Demo', [{ path: 'main.swift', source: 'a' }]);
  const p2 = updateSource(p, 'main.swift', 'b');
  eq(p.files[0].source, 'a', 'original untouched');
  eq(p2.files[0].source, 'b');
  eq(updateSource(p, 'nope.swift', 'x'), p, 'unknown path is identity');
});

check('sortFiles ranks Package.swift then main.swift then alpha', () => {
  const sorted = sortFiles([
    { path: 'Zebra.swift' },
    { path: 'main.swift' },
    { path: 'Package.swift' },
    { path: 'Apple.swift' },
  ]).map((f) => f.path);
  eq(sorted, ['Package.swift', 'main.swift', 'Apple.swift', 'Zebra.swift']);
});

check('serialize/deserialize round-trips; junk returns null', () => {
  const p = createProject('Demo', [{ path: 'main.swift', source: 'print(1)' }]);
  const back = deserialize(serialize(p));
  eq(back.name, 'Demo');
  eq(back.files[0].source, 'print(1)');
  eq(deserialize('not json'), null);
  eq(deserialize(''), null);
  eq(deserialize(JSON.stringify({ version: 999, project: p })), null);
  eq(deserialize(JSON.stringify({ version: 1, project: { name: 'x', files: [] } })), null);
});

check('deserialize repairs a dangling activePath', () => {
  const bad = JSON.stringify({
    version: 1,
    project: { name: 'x', files: [{ path: 'a.swift', source: '' }], activePath: 'gone.swift' },
  });
  const p = deserialize(bad);
  eq(p.activePath, 'a.swift');
});

check('deserialize rejects structurally-invalid file paths', () => {
  const badPath = JSON.stringify({
    version: 1,
    project: { name: 'x', files: [{ path: 'notes.txt', source: '' }], activePath: 'notes.txt' },
  });
  eq(deserialize(badPath), null, 'non-.swift path should be rejected');

  const doubleSlash = JSON.stringify({
    version: 1,
    project: { name: 'x', files: [{ path: 'a//b.swift', source: '' }], activePath: 'a//b.swift' },
  });
  eq(deserialize(doubleSlash), null, 'malformed path should be rejected');

  const nestedManifest = JSON.stringify({
    version: 1,
    project: { name: 'x', files: [{ path: 'Sources/Package.swift', source: '' }], activePath: 'Sources/Package.swift' },
  });
  eq(deserialize(nestedManifest), null, 'Package.swift must be at the root');
});

check('deserialize rejects duplicate file paths', () => {
  const dup = JSON.stringify({
    version: 1,
    project: {
      name: 'x',
      files: [
        { path: 'main.swift', source: 'a' },
        { path: 'main.swift', source: 'b' },
      ],
      activePath: 'main.swift',
    },
  });
  eq(deserialize(dup), null, 'duplicate paths should be rejected');
});

console.log('\nmodule / entry selection');

check('moduleFiles drops Package.swift and leads with the entry file', () => {
  const p = createProject('Demo', [
    { path: 'Package.swift', source: '// manifest' },
    { path: 'Helper.swift', source: 'struct H {}' },
    { path: 'main.swift', source: 'print(1)' },
  ]);
  const files = moduleFiles(p);
  assert(!files.some((f) => f.path === 'Package.swift'), 'manifest excluded');
  eq(files[0].path, 'main.swift', 'entry leads');
  assert('contents' in files[0], 'uses the wasm `contents` key');
});

check('entryFile prefers @main, then main.swift, then top-level code', () => {
  eq(
    entryFile(createProject('D', [
      { path: 'a.swift', source: '@main struct App {}' },
      { path: 'main.swift', source: '' },
    ])),
    'a.swift',
  );
  eq(
    entryFile(createProject('D', [
      { path: 'main.swift', source: 'print(1)' },
      { path: 'b.swift', source: 'struct B {}' },
    ])),
    'main.swift',
  );
  eq(
    entryFile(createProject('D', [
      { path: 'run.swift', source: 'let x = 1\nprint(x)' },
      { path: 'b.swift', source: 'struct B {}' },
    ])),
    'run.swift',
  );
});

check('isSwiftUIProject detects View / some View / App', () => {
  assert(isSwiftUIProject(createProject('D', [{ path: 'V.swift', source: 'struct V: View { var body: some View { Text("x") } }' }])));
  assert(!isSwiftUIProject(createProject('D', [{ path: 'm.swift', source: 'print("hi")' }])));
  assert(declaresView('struct A: App {}'));
  assert(!declaresView('struct Plain { let x = 1 }'));
});

console.log('\noutline / quick-open');

const SYMS = [
  { name: 'Item', kind: 'class', file: 'Item.swift', line: 3 },
  { name: 'title', kind: 'var', file: 'Item.swift', line: 4, container: 'Item' },
  { name: 'done', kind: 'var', file: 'Item.swift', line: 5, container: 'Item' },
  { name: 'ContentView', kind: 'struct', file: 'ContentView.swift', line: 4 },
  { name: 'body', kind: 'var', file: 'ContentView.swift', line: 8, container: 'ContentView' },
];

check('buildOutline nests members under their container, per file', () => {
  const outline = buildOutline(SYMS);
  eq(outline.map((f) => f.file), ['ContentView.swift', 'Item.swift']);
  const item = outline.find((f) => f.file === 'Item.swift');
  eq(item.roots.length, 1);
  eq(item.roots[0].name, 'Item');
  eq(item.roots[0].children.map((c) => c.name), ['title', 'done']);
});

check('quickOpenIndex lists files and symbols with jump targets', () => {
  const p = createProject('D', [
    { path: 'Item.swift', source: '' },
    { path: 'ContentView.swift', source: '' },
  ]);
  const idx = quickOpenIndex(p, SYMS);
  const fileEntries = idx.filter((e) => e.type === 'file');
  eq(fileEntries.length, 2);
  const title = idx.find((e) => e.type === 'symbol' && e.label === 'title');
  eq(title.file, 'Item.swift');
  eq(title.line, 4);
});

check('filterEntries fuzzy-ranks: prefix beats substring beats scatter', () => {
  const p = createProject('D', [{ path: 'ContentView.swift', source: '' }]);
  const idx = quickOpenIndex(p, SYMS);
  const res = filterEntries(idx, 'cont');
  assert(res.length > 0, 'has results');
  eq(res[0].label, 'ContentView', 'prefix match ranks first');
  eq(filterEntries(idx, 'zzzzz').length, 0, 'no match => empty');
});

check('fuzzyScore: prefix > substring > subsequence > miss', () => {
  const prefix = fuzzyScore('contentview', 'cont');
  const substr = fuzzyScore('mycontent', 'cont');
  const subseq = fuzzyScore('creativenotebook', 'cont');
  assert(prefix > substr, `prefix ${prefix} > substr ${substr}`);
  assert(substr > subseq, `substr ${substr} > subseq ${subseq}`);
  assert(fuzzyScore('abc', 'xyz') === -1, 'miss is -1');
});

console.log('\nsamples');

check('every sample builds a valid project with unique .swift paths', () => {
  for (const s of SAMPLES) {
    const p = createProject(s.name, s.files);
    assert(p.files.length >= 1, `${s.id} has files`);
    const paths = p.files.map((f) => f.path);
    eq(paths.length, new Set(paths).size, `${s.id} has unique paths`);
    for (const f of p.files) {
      assert(f.path.endsWith('.swift'), `${s.id}: ${f.path} is .swift`);
    }
  }
});

check('SwiftUI todo sample is recognised as a SwiftUI project', () => {
  const todo = sampleById('swiftui-todo');
  const p = createProject(todo.name, todo.files);
  assert(isSwiftUIProject(p), 'todo renders as SwiftUI');
});

check('console sample ships a Package.swift excluded from the module', () => {
  const c = sampleById('console');
  const p = createProject(c.name, c.files);
  assert(!isSwiftUIProject(p), 'console is not SwiftUI');
  assert(p.files.some((f) => f.path === 'Package.swift'), 'ships a manifest');
  assert(!moduleFiles(p).some((f) => f.path === 'Package.swift'), 'manifest excluded from module');
  assert(moduleFiles(p)[0].path === 'Sources/main.swift', 'nested entry leads the module');
});

if (failures > 0) {
  console.error(`\n${failures} studio check(s) failed`);
  process.exit(1);
}
console.log('\nall studio checks passed');
