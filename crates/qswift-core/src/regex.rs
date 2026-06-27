//! A small, self-contained regular-expression engine.
//!
//! quick-swift builds offline with no access to crates.io (see
//! `docs/agents/environment.md`), so Swift's `Regex` is backed by this
//! hand-written engine rather than the `regex` crate. It parses a pattern into
//! an AST, compiles it to a backtracking byte-code program ([`Inst`]), and
//! matches against a `&[char]` haystack so Unicode scalars are handled
//! correctly.
//!
//! Supported syntax (a pragmatic subset of Swift/PCRE):
//! - literals, `.` (any char except newline)
//! - escapes `\d \D \w \W \s \S \b \B \n \t \r \\` and escaped metacharacters
//! - character classes `[abc]`, ranges `[a-z]`, negation `[^\d]`, class escapes
//! - quantifiers `*` `+` `?` `{n}` `{n,}` `{n,m}`, each optionally lazy (`*?`)
//! - groups `( … )` (capturing) and `(?: … )` (non-capturing)
//! - alternation `a|b`
//! - anchors `^` `$`
//!
//! Matching is leftmost with greedy/lazy priority (Perl semantics), via a
//! recursive backtracking executor over the compiled program.

use std::rc::Rc;

/// Upper bound on a counted-quantifier repeat (`{n}` / `{n,m}`). Larger counts
/// are rejected so a pattern cannot expand into an unbounded amount of bytecode.
const MAX_REPEAT: usize = 1000;

/// A compiled regular expression: its source pattern, byte-code program, and
/// capture-group count (group 0 is the whole match).
#[derive(Debug, Clone)]
pub struct Regex {
    pattern: String,
    prog: Rc<Vec<Inst>>,
    /// Number of capture groups, including the implicit whole-match group 0.
    group_count: usize,
}

/// A successful match: the half-open `[start, end)` span of each capture group
/// (in `char` indices), or `None` for a group that did not participate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captures {
    pub groups: Vec<Option<(usize, usize)>>,
}

impl Captures {
    /// The whole-match span (group 0).
    pub fn whole(&self) -> (usize, usize) {
        self.groups[0].expect("group 0 always participates in a match")
    }
}

impl Regex {
    /// Compile `pattern`, returning a human-readable error on invalid syntax.
    pub fn compile(pattern: &str) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut parser = Parser {
            chars: &chars,
            pos: 0,
            group_count: 1,
        };
        let ast = parser.parse_alternation()?;
        if parser.pos != parser.chars.len() {
            return Err(format!(
                "unexpected '{}' in regex",
                parser.chars[parser.pos]
            ));
        }
        let group_count = parser.group_count;
        let mut prog = Vec::new();
        prog.push(Inst::Save(0));
        compile_node(&ast, &mut prog);
        prog.push(Inst::Save(1));
        prog.push(Inst::Match);
        Ok(Regex {
            pattern: pattern.to_string(),
            prog: Rc::new(prog),
            group_count,
        })
    }

    /// The original pattern text (without delimiters).
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Find the leftmost match at or after `from` (a `char` index). Returns the
    /// capture spans, or `None` if the pattern does not match anywhere.
    pub fn find_at(&self, input: &[char], from: usize) -> Option<Captures> {
        for start in from..=input.len() {
            let mut saves = vec![None; self.group_count * 2];
            if exec(&self.prog, 0, input, start, &mut saves) {
                return Some(self.captures(&saves));
            }
        }
        None
    }

    /// Find the leftmost match anywhere in `input`.
    pub fn find(&self, input: &[char]) -> Option<Captures> {
        self.find_at(input, 0)
    }

    /// Match anchored at the start of `input` (Swift's `prefixMatch(of:)`).
    pub fn prefix_match(&self, input: &[char]) -> Option<Captures> {
        let mut saves = vec![None; self.group_count * 2];
        if exec(&self.prog, 0, input, 0, &mut saves) {
            Some(self.captures(&saves))
        } else {
            None
        }
    }

    /// Match the *entire* `input` (Swift's `wholeMatch(of:)`).
    pub fn whole_match(&self, input: &[char]) -> Option<Captures> {
        let m = self.prefix_match(input)?;
        if m.whole() == (0, input.len()) {
            Some(m)
        } else {
            None
        }
    }

    /// All non-overlapping matches, left to right (Swift's `matches(of:)`).
    pub fn find_all(&self, input: &[char]) -> Vec<Captures> {
        let mut out = Vec::new();
        let mut from = 0;
        while from <= input.len() {
            let Some(m) = self.find_at(input, from) else {
                break;
            };
            let (s, e) = m.whole();
            out.push(m);
            // Advance past the match; step one char on an empty match to make
            // progress and avoid an infinite loop.
            from = if e > s { e } else { e + 1 };
        }
        out
    }

    fn captures(&self, saves: &[Option<usize>]) -> Captures {
        let groups = (0..self.group_count)
            .map(|g| match (saves[g * 2], saves[g * 2 + 1]) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => None,
            })
            .collect();
        Captures { groups }
    }
}

impl PartialEq for Regex {
    /// Two regexes are equal when their source patterns are equal.
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
    }
}

// --- AST ---------------------------------------------------------------------

#[derive(Debug)]
enum Node {
    Empty,
    Char(char),
    AnyChar,
    Class {
        items: Vec<ClassItem>,
        negated: bool,
    },
    Start,
    End,
    WordBoundary(bool),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: Option<usize>,
        greedy: bool,
    },
    Group {
        index: Option<usize>,
        node: Box<Node>,
    },
}

#[derive(Debug, Clone)]
enum ClassItem {
    Char(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

// --- parser ------------------------------------------------------------------

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
    group_count: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn parse_alternation(&mut self) -> Result<Node, String> {
        let mut branches = vec![self.parse_concat()?];
        while self.peek() == Some('|') {
            self.bump();
            branches.push(self.parse_concat()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(Node::Alt(branches))
        }
    }

    fn parse_concat(&mut self) -> Result<Node, String> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            nodes.push(self.parse_repeat()?);
        }
        match nodes.len() {
            0 => Ok(Node::Empty),
            1 => Ok(nodes.pop().unwrap()),
            _ => Ok(Node::Concat(nodes)),
        }
    }

    fn parse_repeat(&mut self) -> Result<Node, String> {
        let atom = self.parse_atom()?;
        let (min, max) = match self.peek() {
            Some('*') => {
                self.bump();
                (0, None)
            }
            Some('+') => {
                self.bump();
                (1, None)
            }
            Some('?') => {
                self.bump();
                (0, Some(1))
            }
            Some('{') => match self.parse_counted()? {
                Some(bounds) => bounds,
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        let greedy = if self.peek() == Some('?') {
            self.bump();
            false
        } else {
            true
        };
        Ok(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    /// Parse a `{n}` / `{n,}` / `{n,m}` quantifier. Returns `Ok(None)` to treat
    /// a `{` that is not a valid counted quantifier as a literal brace, and
    /// `Err` for a well-formed but invalid range (`max < min`) or counts that
    /// would expand to an unreasonable amount of bytecode.
    fn parse_counted(&mut self) -> Result<Option<(usize, Option<usize>)>, String> {
        let save = self.pos;
        self.bump(); // '{'
        let min = self.parse_int();
        let result = match self.peek() {
            Some('}') if min.is_some() => {
                self.bump();
                Some((min.unwrap(), Some(min.unwrap())))
            }
            Some(',') => {
                self.bump();
                let max = self.parse_int();
                if self.peek() == Some('}') {
                    self.bump();
                    Some((min.unwrap_or(0), max))
                } else {
                    None
                }
            }
            _ => None,
        };
        match result {
            None => {
                self.pos = save; // not a quantifier; rewind so `{` is literal
                Ok(None)
            }
            Some((min, max)) => {
                if let Some(max) = max {
                    if max < min {
                        return Err(format!("invalid quantifier range {{{min},{max}}}"));
                    }
                }
                if min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT) {
                    return Err(format!(
                        "quantifier count exceeds the maximum of {MAX_REPEAT}"
                    ));
                }
                Ok(Some((min, max)))
            }
        }
    }

    fn parse_int(&mut self) -> Option<usize> {
        let start = self.pos;
        let mut n: usize = 0;
        while let Some(c) = self.peek() {
            if let Some(d) = c.to_digit(10) {
                // Saturate rather than overflow; `parse_counted` rejects counts
                // above `MAX_REPEAT` anyway.
                n = n.saturating_mul(10).saturating_add(d as usize);
                self.bump();
            } else {
                break;
            }
        }
        if self.pos == start {
            None
        } else {
            Some(n)
        }
    }

    fn parse_atom(&mut self) -> Result<Node, String> {
        match self.peek() {
            Some('(') => self.parse_group(),
            Some('[') => self.parse_class(),
            Some('.') => {
                self.bump();
                Ok(Node::AnyChar)
            }
            Some('^') => {
                self.bump();
                Ok(Node::Start)
            }
            Some('$') => {
                self.bump();
                Ok(Node::End)
            }
            Some('\\') => self.parse_escape(),
            Some(c) if c == '*' || c == '+' || c == '?' => {
                Err(format!("dangling quantifier '{c}'"))
            }
            Some(c) => {
                self.bump();
                Ok(Node::Char(c))
            }
            None => Ok(Node::Empty),
        }
    }

    fn parse_group(&mut self) -> Result<Node, String> {
        self.bump(); // '('
        let mut index = Some(self.group_count);
        if self.peek() == Some('?') {
            // Only the non-capturing form `(?: … )` is supported.
            if self.chars.get(self.pos + 1) == Some(&':') {
                self.pos += 2;
                index = None;
            } else {
                return Err("unsupported group extension '(?'".to_string());
            }
        }
        if let Some(i) = index {
            self.group_count = i + 1;
        }
        let inner = self.parse_alternation()?;
        if self.peek() != Some(')') {
            return Err("missing ')' in regex".to_string());
        }
        self.bump(); // ')'
        Ok(Node::Group {
            index,
            node: Box::new(inner),
        })
    }

    fn parse_class(&mut self) -> Result<Node, String> {
        self.bump(); // '['
        let negated = if self.peek() == Some('^') {
            self.bump();
            true
        } else {
            false
        };
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == ']' {
                self.bump();
                return Ok(Node::Class { items, negated });
            }
            let lo = self.class_char()?;
            // A range `a-z` (but a trailing `-` before `]` is a literal dash).
            if let ClassAtom::Char(lo_c) = lo {
                if self.peek() == Some('-') && self.chars.get(self.pos + 1) != Some(&']') {
                    self.bump(); // '-'
                    match self.class_char()? {
                        ClassAtom::Char(hi_c) => {
                            items.push(ClassItem::Range(lo_c, hi_c));
                            continue;
                        }
                        _ => return Err("invalid range in character class".to_string()),
                    }
                }
                items.push(ClassItem::Char(lo_c));
            } else if let ClassAtom::Item(item) = lo {
                items.push(item);
            }
        }
        Err("unterminated character class".to_string())
    }

    /// Parse one element inside a `[ … ]` class: a literal char or a class
    /// escape such as `\d`.
    fn class_char(&mut self) -> Result<ClassAtom, String> {
        match self.bump() {
            Some('\\') => match self.bump() {
                Some('d') => Ok(ClassAtom::Item(ClassItem::Digit(true))),
                Some('D') => Ok(ClassAtom::Item(ClassItem::Digit(false))),
                Some('w') => Ok(ClassAtom::Item(ClassItem::Word(true))),
                Some('W') => Ok(ClassAtom::Item(ClassItem::Word(false))),
                Some('s') => Ok(ClassAtom::Item(ClassItem::Space(true))),
                Some('S') => Ok(ClassAtom::Item(ClassItem::Space(false))),
                Some('n') => Ok(ClassAtom::Char('\n')),
                Some('t') => Ok(ClassAtom::Char('\t')),
                Some('r') => Ok(ClassAtom::Char('\r')),
                Some(c) => Ok(ClassAtom::Char(c)),
                None => Err("trailing '\\' in character class".to_string()),
            },
            Some(c) => Ok(ClassAtom::Char(c)),
            None => Err("unterminated character class".to_string()),
        }
    }

    fn parse_escape(&mut self) -> Result<Node, String> {
        self.bump(); // '\'
        match self.bump() {
            Some('d') => Ok(class(ClassItem::Digit(true), false)),
            Some('D') => Ok(class(ClassItem::Digit(true), true)),
            Some('w') => Ok(class(ClassItem::Word(true), false)),
            Some('W') => Ok(class(ClassItem::Word(true), true)),
            Some('s') => Ok(class(ClassItem::Space(true), false)),
            Some('S') => Ok(class(ClassItem::Space(true), true)),
            Some('b') => Ok(Node::WordBoundary(true)),
            Some('B') => Ok(Node::WordBoundary(false)),
            Some('n') => Ok(Node::Char('\n')),
            Some('t') => Ok(Node::Char('\t')),
            Some('r') => Ok(Node::Char('\r')),
            Some('0') => Ok(Node::Char('\0')),
            Some(c) => Ok(Node::Char(c)),
            None => Err("trailing '\\' in regex".to_string()),
        }
    }
}

enum ClassAtom {
    Char(char),
    Item(ClassItem),
}

fn class(item: ClassItem, negated: bool) -> Node {
    Node::Class {
        items: vec![item],
        negated,
    }
}

// --- compiler ----------------------------------------------------------------

/// A backtracking byte-code instruction.
#[derive(Debug)]
enum Inst {
    Char(char),
    AnyChar,
    Class {
        items: Vec<ClassItem>,
        negated: bool,
    },
    Start,
    End,
    WordBoundary(bool),
    /// Record the current position into save slot `n` (capture boundary).
    Save(usize),
    Jmp(usize),
    /// Try `x` first; on failure, fall through to `y` (priority ordering).
    Split(usize, usize),
    Match,
}

fn compile_node(node: &Node, prog: &mut Vec<Inst>) {
    match node {
        Node::Empty => {}
        Node::Char(c) => prog.push(Inst::Char(*c)),
        Node::AnyChar => prog.push(Inst::AnyChar),
        Node::Start => prog.push(Inst::Start),
        Node::End => prog.push(Inst::End),
        Node::WordBoundary(b) => prog.push(Inst::WordBoundary(*b)),
        Node::Class { items, negated } => prog.push(Inst::Class {
            items: items.clone(),
            negated: *negated,
        }),
        Node::Concat(nodes) => {
            for n in nodes {
                compile_node(n, prog);
            }
        }
        Node::Group { index, node } => {
            if let Some(i) = index {
                prog.push(Inst::Save(i * 2));
                compile_node(node, prog);
                prog.push(Inst::Save(i * 2 + 1));
            } else {
                compile_node(node, prog);
            }
        }
        Node::Alt(branches) => compile_alt(branches, prog),
        Node::Repeat {
            node,
            min,
            max,
            greedy,
        } => compile_repeat(node, *min, *max, *greedy, prog),
    }
}

fn compile_alt(branches: &[Node], prog: &mut Vec<Inst>) {
    if branches.len() == 1 {
        compile_node(&branches[0], prog);
        return;
    }
    // Split into the first branch vs the rest; collect jumps to the end.
    let split = prog.len();
    prog.push(Inst::Split(0, 0));
    let l1 = prog.len();
    compile_node(&branches[0], prog);
    let jmp = prog.len();
    prog.push(Inst::Jmp(0));
    let l2 = prog.len();
    if let Inst::Split(a, b) = &mut prog[split] {
        *a = l1;
        *b = l2;
    }
    compile_alt(&branches[1..], prog);
    let end = prog.len();
    if let Inst::Jmp(target) = &mut prog[jmp] {
        *target = end;
    }
}

fn compile_repeat(node: &Node, min: usize, max: Option<usize>, greedy: bool, prog: &mut Vec<Inst>) {
    // Emit `min` mandatory copies.
    for _ in 0..min {
        compile_node(node, prog);
    }
    match max {
        None => {
            // Unbounded tail: `L1: split body,end; body; jmp L1; end:`
            let l1 = prog.len();
            let split = prog.len();
            prog.push(Inst::Split(0, 0));
            let body = prog.len();
            compile_node(node, prog);
            prog.push(Inst::Jmp(l1));
            let end = prog.len();
            patch_split(prog, split, body, end, greedy);
        }
        Some(max) => {
            // Emit `max - min` optional copies, each guarded by a split.
            let mut splits = Vec::new();
            for _ in min..max {
                let split = prog.len();
                prog.push(Inst::Split(0, 0));
                let body = prog.len();
                compile_node(node, prog);
                splits.push((split, body));
            }
            let end = prog.len();
            for (split, body) in splits {
                patch_split(prog, split, body, end, greedy);
            }
        }
    }
}

/// Fill in a `Split` so the preferred branch (`body` when greedy, `skip` when
/// lazy) is tried first.
fn patch_split(prog: &mut [Inst], at: usize, body: usize, skip: usize, greedy: bool) {
    if let Inst::Split(a, b) = &mut prog[at] {
        if greedy {
            *a = body;
            *b = skip;
        } else {
            *a = skip;
            *b = body;
        }
    }
}

// --- executor ----------------------------------------------------------------

/// Recursive backtracking executor. Runs `prog` from `pc` against `input`
/// starting at `pos`, recording capture boundaries in `saves`. Returns whether
/// a match was found (with `saves` populated on success).
fn exec(
    prog: &[Inst],
    pc: usize,
    input: &[char],
    pos: usize,
    saves: &mut Vec<Option<usize>>,
) -> bool {
    // `active` holds the `(split_pc, pos)` states currently on the recursion
    // stack, so an empty-width quantifier body (e.g. `(a?)*`, `()*`) cannot
    // re-enter the same split at the same position and recurse forever.
    let mut active = std::collections::HashSet::new();
    exec_inner(prog, pc, input, pos, saves, &mut active)
}

fn exec_inner(
    prog: &[Inst],
    mut pc: usize,
    input: &[char],
    mut pos: usize,
    saves: &mut Vec<Option<usize>>,
    active: &mut std::collections::HashSet<(usize, usize)>,
) -> bool {
    loop {
        match &prog[pc] {
            Inst::Char(c) => {
                if pos < input.len() && input[pos] == *c {
                    pos += 1;
                    pc += 1;
                } else {
                    return false;
                }
            }
            Inst::AnyChar => {
                if pos < input.len() && input[pos] != '\n' {
                    pos += 1;
                    pc += 1;
                } else {
                    return false;
                }
            }
            Inst::Class { items, negated } => {
                if pos < input.len() && class_matches(items, *negated, input[pos]) {
                    pos += 1;
                    pc += 1;
                } else {
                    return false;
                }
            }
            Inst::Start => {
                if pos == 0 || input[pos - 1] == '\n' {
                    pc += 1;
                } else {
                    return false;
                }
            }
            Inst::End => {
                if pos == input.len() || input[pos] == '\n' {
                    pc += 1;
                } else {
                    return false;
                }
            }
            Inst::WordBoundary(want) => {
                let before = pos > 0 && is_word(input[pos - 1]);
                let after = pos < input.len() && is_word(input[pos]);
                if (before != after) == *want {
                    pc += 1;
                } else {
                    return false;
                }
            }
            Inst::Save(slot) => {
                let prev = saves[*slot];
                saves[*slot] = Some(pos);
                if exec_inner(prog, pc + 1, input, pos, saves, active) {
                    return true;
                }
                saves[*slot] = prev; // backtrack: restore the slot
                return false;
            }
            Inst::Jmp(target) => pc = *target,
            Inst::Split(a, b) => {
                let key = (pc, pos);
                // Re-entering the same split at the same position means the body
                // made no progress; take the exit branch instead of looping.
                if active.contains(&key) {
                    pc = *b;
                    continue;
                }
                active.insert(key);
                let matched = exec_inner(prog, *a, input, pos, saves, active);
                active.remove(&key);
                if matched {
                    return true;
                }
                pc = *b;
            }
            Inst::Match => return true,
        }
    }
}

fn class_matches(items: &[ClassItem], negated: bool, c: char) -> bool {
    let mut hit = false;
    for item in items {
        let m = match item {
            ClassItem::Char(x) => *x == c,
            ClassItem::Range(lo, hi) => *lo <= c && c <= *hi,
            ClassItem::Digit(pos) => c.is_ascii_digit() == *pos,
            ClassItem::Word(pos) => is_word(c) == *pos,
            ClassItem::Space(pos) => c.is_whitespace() == *pos,
        };
        if m {
            hit = true;
            break;
        }
    }
    hit ^ negated
}

fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn matched(re: &Regex, hay: &str) -> Option<String> {
        let cs = chars(hay);
        re.find(&cs).map(|m| {
            let (s, e) = m.whole();
            cs[s..e].iter().collect()
        })
    }

    #[test]
    fn literal_and_anchors() {
        let re = Regex::compile("abc").unwrap();
        assert_eq!(matched(&re, "xabcy").as_deref(), Some("abc"));
        let re = Regex::compile("^abc$").unwrap();
        assert!(re.whole_match(&chars("abc")).is_some());
        assert!(re.find(&chars("xabc")).is_none());
    }

    #[test]
    fn digit_and_quantifiers() {
        let re = Regex::compile(r"\d+").unwrap();
        assert_eq!(matched(&re, "abc123def").as_deref(), Some("123"));
        let re = Regex::compile(r"a*").unwrap();
        assert_eq!(matched(&re, "aaab").as_deref(), Some("aaa"));
        let re = Regex::compile(r"a{2,3}").unwrap();
        assert_eq!(matched(&re, "aaaa").as_deref(), Some("aaa"));
    }

    #[test]
    fn lazy_quantifier() {
        let re = Regex::compile(r"a+?").unwrap();
        assert_eq!(matched(&re, "aaa").as_deref(), Some("a"));
    }

    #[test]
    fn counted_quantifier_validation() {
        // Well-formed range works.
        assert!(Regex::compile(r"a{2,3}").is_ok());
        // max < min is rejected.
        assert!(Regex::compile(r"a{3,2}").is_err());
        // Excessive counts are rejected rather than expanded.
        assert!(Regex::compile(r"a{100000}").is_err());
        assert!(Regex::compile(r"a{0,100000}").is_err());
        // A `{` that is not a quantifier stays a literal brace.
        assert_eq!(
            matched(&Regex::compile(r"a{b").unwrap(), "a{b").as_deref(),
            Some("a{b")
        );
    }

    #[test]
    fn nullable_quantifier_does_not_loop_forever() {
        // Empty-width quantifier bodies must terminate rather than recurse
        // forever / overflow the stack.
        let re = Regex::compile(r"(a?)*").unwrap();
        assert_eq!(matched(&re, "aaab").as_deref(), Some("aaa"));
        let re = Regex::compile(r"()*").unwrap();
        assert_eq!(matched(&re, "xyz").as_deref(), Some(""));
        let re = Regex::compile(r"(a*)*b").unwrap();
        assert_eq!(matched(&re, "aaab").as_deref(), Some("aaab"));
        let re = Regex::compile(r"(a*)*c").unwrap();
        assert!(re.find(&chars("aaab")).is_none());
    }

    #[test]
    fn character_class_and_ranges() {
        let re = Regex::compile(r"[a-c]+").unwrap();
        assert_eq!(matched(&re, "abcd").as_deref(), Some("abc"));
        let re = Regex::compile(r"[^0-9]+").unwrap();
        assert_eq!(matched(&re, "ab12").as_deref(), Some("ab"));
    }

    #[test]
    fn alternation_and_groups() {
        let re = Regex::compile(r"(cat|dog)s?").unwrap();
        assert_eq!(matched(&re, "two dogs").as_deref(), Some("dogs"));
    }

    #[test]
    fn capture_groups() {
        let re = Regex::compile(r"(\d+)-(\d+)").unwrap();
        let cs = chars("12-345");
        let m = re.find(&cs).unwrap();
        assert_eq!(m.groups[1], Some((0, 2)));
        assert_eq!(m.groups[2], Some((3, 6)));
    }

    #[test]
    fn find_all_non_overlapping() {
        let re = Regex::compile(r"\d+").unwrap();
        let cs = chars("a1b22c333");
        let all: Vec<String> = re
            .find_all(&cs)
            .iter()
            .map(|m| {
                let (s, e) = m.whole();
                cs[s..e].iter().collect()
            })
            .collect();
        assert_eq!(all, vec!["1", "22", "333"]);
    }

    #[test]
    fn word_boundary() {
        let re = Regex::compile(r"\bcat\b").unwrap();
        assert!(re.find(&chars("the cat sat")).is_some());
        assert!(re.find(&chars("category")).is_none());
    }

    #[test]
    fn invalid_patterns_error() {
        assert!(Regex::compile("(unclosed").is_err());
        assert!(Regex::compile("a**").is_err());
    }

    #[test]
    fn whole_and_prefix_match() {
        let re = Regex::compile(r"\d+").unwrap();
        assert!(re.whole_match(&chars("123")).is_some());
        assert!(re.whole_match(&chars("123a")).is_none());
        assert!(re.prefix_match(&chars("123a")).is_some());
        assert!(re.prefix_match(&chars("a123")).is_none());
    }
}
