//! Quint counterexamples drawn on the spec that produced them.
//!
//! A Quint run that violates an invariant emits an ITF trace: an ordered list
//! of states, each binding every state variable. The spec itself is already a
//! tile on the map, so a trace needs no new geometry — it is an overlay whose
//! marks move as the trace is stepped. The lines that light up are where each
//! variable is declared and where it is assigned, so stepping a counterexample
//! walks the reader through the part of the spec that produced it.

use crate::overlay::{FileOverlay, FileState, Overlay, M_NONE, M_PAST, M_STEP};
use crate::scan::SourceFile;

// ---------------------------------------------------------------- JSON

/// Just enough JSON for ITF. Hand-rolled to keep this crate dependency-free,
/// like the source lexer and the atlas packer next to it.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.i < self.b.len() && self.b[self.i] == c {
            self.i += 1;
            return true;
        }
        false
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') {
            return None;
        }
        let mut s = String::new();
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'"' => {
                    self.i += 1;
                    return Some(s);
                }
                b'\\' => {
                    self.i += 1;
                    let c = *self.b.get(self.i)?;
                    self.i += 1;
                    match c {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'b' => s.push('\u{8}'),
                        b'f' => s.push('\u{c}'),
                        b'u' => {
                            let hex = self.b.get(self.i..self.i + 4)?;
                            self.i += 4;
                            let n = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            s.push(char::from_u32(n).unwrap_or('?'));
                        }
                        other => s.push(other as char),
                    }
                }
                _ => {
                    // Copy the whole UTF-8 sequence.
                    let start = self.i;
                    self.i += 1;
                    while self.i < self.b.len() && self.b[self.i] & 0xC0 == 0x80 {
                        self.i += 1;
                    }
                    s.push_str(std::str::from_utf8(&self.b[start..self.i]).ok()?);
                }
            }
        }
        None
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.b.get(self.i)? {
            b'"' => self.string().map(Json::Str),
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                if self.eat(b'}') {
                    return Some(Json::Obj(fields));
                }
                loop {
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    fields.push((k, self.value()?));
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b'}').then_some(Json::Obj(fields));
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b']').then_some(Json::Arr(items));
                }
            }
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'n' => self.lit("null", Json::Null),
            _ => {
                let start = self.i;
                while self.i < self.b.len()
                    && matches!(
                        self.b[self.i],
                        b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'
                    )
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i])
                    .ok()?
                    .parse()
                    .ok()
                    .map(Json::Num)
            }
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Option<Json> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            return Some(v);
        }
        None
    }
}

pub fn parse_json(text: &str) -> Option<Json> {
    P {
        b: text.as_bytes(),
        i: 0,
    }
    .value()
}

/// Renders an ITF value the way Quint prints it.
pub fn render(v: &Json) -> String {
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => {
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Json::Str(s) => s.clone(),
        Json::Arr(a) => {
            let items: Vec<String> = a.iter().map(render).collect();
            format!("[{}]", items.join(", "))
        }
        Json::Obj(fields) => {
            // ITF tags: #bigint, #set, #map, #tup are wrappers, not records.
            if let Some(Json::Str(n)) = v.get("#bigint") {
                return n.clone();
            }
            if let Some(Json::Arr(items)) = v.get("#set") {
                let items: Vec<String> = items.iter().map(render).collect();
                return format!("Set({})", items.join(", "));
            }
            if let Some(Json::Arr(items)) = v.get("#tup") {
                let items: Vec<String> = items.iter().map(render).collect();
                return format!("({})", items.join(", "));
            }
            if let Some(Json::Arr(pairs)) = v.get("#map") {
                let items: Vec<String> = pairs
                    .iter()
                    .map(|p| match p.as_array() {
                        Some([k, val]) => format!("{} -> {}", render(k), render(val)),
                        _ => render(p),
                    })
                    .collect();
                return format!("Map({})", items.join(", "));
            }
            let items: Vec<String> = fields
                .iter()
                .filter(|(k, _)| !k.starts_with('#'))
                .map(|(k, val)| format!("{k}: {}", render(val)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

// ---------------------------------------------------------------- trace

pub struct Trace {
    /// Spec path as recorded in the trace, relative to the repository root.
    pub source: String,
    pub status: String,
    pub vars: Vec<String>,
    /// Rendered value per variable, in `vars` order, for each state.
    pub states: Vec<Vec<String>>,
    /// Index into `Scene::sources` once bound, and the lines each var owns.
    pub file: Option<usize>,
    pub var_lines: Vec<Vec<usize>>,
    pub step: usize,
    pub playing: bool,
    /// Seconds spent on the current step.
    pub elapsed: f64,
    /// Seconds each step is held during playback.
    pub step_secs: f64,
    /// Wrap to the first state after the last. A recording plays once instead.
    pub looping: bool,
}

/// Seconds a step is held during playback, unless `--trace-rate` says otherwise.
pub const STEP_SECS: f64 = 1.6;

/// How long a trace that plays once stays on its last state before it is done.
pub const END_HOLD_SECS: f64 = 1.5;

impl Trace {
    pub fn parse(text: &str) -> Option<Trace> {
        let j = parse_json(text)?;
        let vars: Vec<String> = j
            .get("vars")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let states = j
            .get("states")?
            .as_array()?
            .iter()
            .map(|s| {
                vars.iter()
                    .map(|v| s.get(v).map(render).unwrap_or_default())
                    .collect()
            })
            .collect();
        let meta = j.get("#meta");
        Some(Trace {
            source: meta
                .and_then(|m| m.get("source"))
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string(),
            status: meta
                .and_then(|m| m.get("status"))
                .and_then(Json::as_str)
                .unwrap_or("unknown")
                .to_string(),
            var_lines: vec![Vec::new(); vars.len()],
            vars,
            states,
            file: None,
            step: 0,
            playing: true,
            elapsed: 0.0,
            step_secs: STEP_SECS,
            looping: true,
        })
    }

    /// Locates the spec among the scanned sources and the lines each variable
    /// owns: its `var` declaration and every line that assigns it (`name'`).
    pub fn bind(&mut self, sources: &[SourceFile]) -> bool {
        let Some(i) = sources
            .iter()
            .position(|s| s.path == self.source || self.source.ends_with(&s.path))
        else {
            return false;
        };
        let src = &sources[i];
        for (v, name) in self.vars.iter().enumerate() {
            for line in 0..src.line_count() {
                let (text, _) = src.line(line);
                if owns(text, name.as_bytes()) {
                    self.var_lines[v].push(line);
                }
            }
        }
        self.file = Some(i);
        true
    }

    /// Variables whose value differs from the previous state.
    pub fn changed(&self, step: usize) -> Vec<usize> {
        if step == 0 || step >= self.states.len() {
            return (0..self.vars.len()).collect();
        }
        (0..self.vars.len())
            .filter(|&v| self.states[step][v] != self.states[step - 1][v])
            .collect()
    }

    /// Rewrites the spec's marks for the current step. Lines belonging to a
    /// variable written now are bright; those written earlier stay dim.
    pub fn apply(&self, ov: &mut Overlay, lines: usize) {
        let Some(file) = self.file else { return };
        let mut marks = vec![M_NONE; lines];
        for past in 0..self.step {
            for v in self.changed(past) {
                for &l in &self.var_lines[v] {
                    if let Some(m) = marks.get_mut(l) {
                        *m = M_PAST;
                    }
                }
            }
        }
        for v in self.changed(self.step) {
            for &l in &self.var_lines[v] {
                if let Some(m) = marks.get_mut(l) {
                    *m = M_STEP;
                }
            }
        }
        ov.files[file] = Some(FileOverlay {
            state: FileState::Modified,
            added: 0,
            removed: 0,
            marks,
        });
    }

    /// Advances playback; returns true when the step changed. A fast rate can
    /// take several steps in one frame, so the remainder carries over.
    pub fn tick(&mut self, dt: f64) -> bool {
        let n = self.states.len();
        if !self.playing || n < 2 {
            return false;
        }
        self.elapsed += dt;
        let due = (self.elapsed / self.step_secs) as usize;
        // Played once, the trace parks on its last state, and `elapsed` goes
        // on to time the hold.
        let steps = if self.looping {
            due
        } else {
            due.min(n - 1 - self.step)
        };
        if steps == 0 {
            return false;
        }
        self.elapsed -= steps as f64 * self.step_secs;
        self.step = (self.step + steps % n) % n;
        true
    }

    /// A trace that plays once is done after holding its last state.
    pub fn finished(&self) -> bool {
        !self.looping && self.step + 1 >= self.states.len() && self.elapsed >= END_HOLD_SECS
    }

    pub fn seek(&mut self, delta: isize) {
        let n = self.states.len() as isize;
        if n == 0 {
            return;
        }
        self.step = (self.step as isize + delta).rem_euclid(n) as usize;
        self.elapsed = 0.0;
    }

    /// One HUD line per variable for the current state.
    pub fn state_lines(&self) -> Vec<(String, bool)> {
        let changed = self.changed(self.step);
        self.vars
            .iter()
            .enumerate()
            .map(|(v, name)| {
                let value = self.states[self.step].get(v).cloned().unwrap_or_default();
                (format!("{name} = {value}"), changed.contains(&v))
            })
            .collect()
    }
}

/// True when `line` is somewhere a change to `name` can come from: its `var`
/// declaration, or an assignment whose right-hand side is not the variable
/// itself. Quint actions must assign every variable, so the identity writes
/// (`best' = best`) that pad every action carry no information and would
/// otherwise light the whole spec on every step.
fn owns(line: &[u8], name: &[u8]) -> bool {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while let Some(p) = line[i..]
        .windows(name.len())
        .position(|w| w == name)
        .map(|p| p + i)
    {
        i = p + name.len();
        if p > 0 && ident(line[p - 1]) {
            continue;
        }
        match line.get(i) {
            // `name' = rhs`
            Some(b'\'') => {
                let mut j = i + 1;
                while line.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                    j += 1;
                }
                if line.get(j) != Some(&b'=') {
                    continue;
                }
                j += 1;
                while line.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                    j += 1;
                }
                let start = j;
                while line.get(j).is_some_and(|c| ident(*c)) {
                    j += 1;
                }
                if &line[start..j] != name {
                    return true;
                }
                // `name' = name` only counts if something follows it.
                let mut k = j;
                while line.get(k).is_some_and(|c| c.is_ascii_whitespace()) {
                    k += 1;
                }
                if !matches!(line.get(k), None | Some(b',') | Some(b'}') | Some(b')')) {
                    return true;
                }
            }
            // `var name: T`
            Some(c) if !ident(*c) => {
                let head = std::str::from_utf8(&line[..p]).unwrap_or("").trim_start();
                if head == "var" || head.starts_with("var ") {
                    return true;
                }
            }
            None => {}
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/channel-payout-liveness.inv_escape_naive_safe.itf.json"
    );

    #[test]
    fn parses_itf_values() {
        assert_eq!(render(&parse_json(r##"{"#bigint":"42"}"##).unwrap()), "42");
        assert_eq!(render(&parse_json("true").unwrap()), "true");
        assert_eq!(
            render(&parse_json(r##"{"#set":[{"#bigint":"1"},{"#bigint":"2"}]}"##).unwrap()),
            "Set(1, 2)"
        );
        assert_eq!(
            render(&parse_json(r##"{"#map":[[{"#bigint":"1"},true]]}"##).unwrap()),
            "Map(1 -> true)"
        );
        assert_eq!(
            render(&parse_json(r##"{"a":1,"#meta":{}}"##).unwrap()),
            "{a: 1}"
        );
    }

    #[test]
    fn reads_a_real_counterexample() {
        let text = std::fs::read_to_string(REAL).expect("checked-in ITF trace");
        let t = Trace::parse(&text).expect("parses");
        assert_eq!(t.status, "violation");
        assert_eq!(t.source, "formal-specs/channel/channel-payout-liveness.qnt");
        assert_eq!(t.states.len(), 6);
        assert!(t.vars.contains(&"paidNaiveEscape".to_string()));
        // The first state binds every variable, so all of them read as changed.
        assert_eq!(t.changed(0).len(), t.vars.len());
        // Somewhere in the trace a single step must flip the unsafe payout.
        let flips = (1..t.states.len())
            .filter(|&s| !t.changed(s).is_empty())
            .count();
        assert!(flips > 0, "a counterexample must change state");
    }

    #[test]
    fn a_variable_owns_its_declaration_and_its_real_writes() {
        assert!(owns(b"  var best: int", b"best"));
        assert!(owns(b"    best' = amount,", b"best"));
        assert!(owns(b"best' = 1", b"best"));
        assert!(owns(b"    best' = best + 1, disputed' = disputed", b"best"));
        // Reads are not assignments, and neither are longer names.
        assert!(!owns(b"    if (best > 0) {", b"best"));
        assert!(!owns(b"    var bestEffort: int", b"best"));
        assert!(!owns(b"    notbest' = 1", b"best"));
        // Quint pads every action with identity writes; they carry nothing.
        assert!(!owns(b"    best' = best, finalized' = finalized,", b"best"));
        assert!(!owns(b"    best' = best }", b"best"));
    }

    #[test]
    fn a_trace_naming_an_unknown_spec_stays_unbound() {
        // Callers key off the return value; `file` must stay None so nothing
        // downstream can unwrap its way into a panic.
        let text = std::fs::read_to_string(REAL).unwrap();
        let mut t = Trace::parse(&text).unwrap();
        assert!(!t.bind(&[]), "no sources can match");
        assert_eq!(t.file, None);
    }

    #[test]
    fn a_fast_rate_takes_several_steps_per_frame() {
        let text = std::fs::read_to_string(REAL).unwrap();
        let mut t = Trace::parse(&text).unwrap();
        // Twice as many steps as frames (64 and 32 a second, which are exact
        // in binary): two steps a frame.
        t.step_secs = 1.0 / 64.0;
        assert!(t.tick(1.0 / 32.0));
        assert_eq!(t.step, 2);
        // The default rate holds each step, and wraps after the last.
        t.step_secs = STEP_SECS;
        t.step = t.states.len() - 1;
        t.elapsed = 0.0;
        assert!(!t.tick(1.0));
        assert!(t.tick(1.0));
        assert_eq!(t.step, 0);
    }

    #[test]
    fn a_trace_played_once_parks_on_its_last_state_then_finishes() {
        let text = std::fs::read_to_string(REAL).unwrap();
        let mut t = Trace::parse(&text).unwrap();
        let last = t.states.len() - 1;
        t.looping = false;
        t.step_secs = 0.1;
        // One long frame runs past the end: it stops on the last state.
        assert!(t.tick(10.0));
        assert_eq!(t.step, last);
        assert!(t.finished(), "held well past END_HOLD_SECS");

        t.step = last - 1;
        t.elapsed = 0.0;
        assert!(t.tick(0.1));
        assert_eq!(t.step, last);
        assert!(!t.finished(), "just arrived; the hold has not run");
        assert!(!t.tick(END_HOLD_SECS));
        assert_eq!(t.step, last);
        assert!(t.finished());
    }

    #[test]
    fn marks_walk_the_spec_as_the_trace_steps() {
        let text = std::fs::read_to_string(REAL).unwrap();
        let mut t = Trace::parse(&text).unwrap();
        t.file = Some(0);
        t.var_lines = vec![vec![1], vec![2], vec![3], vec![4], vec![5]];
        let mut ov = Overlay::empty(1);

        t.step = 0;
        t.apply(&mut ov, 8);
        assert_eq!(ov.mark(0, 1), M_STEP, "state 0 binds every variable");

        // Later steps keep earlier writes visible, but dimmed.
        t.step = t.states.len() - 1;
        t.apply(&mut ov, 8);
        let now = t.changed(t.step);
        for v in 0..t.vars.len() {
            let want = if now.contains(&v) { M_STEP } else { M_PAST };
            assert_eq!(ov.mark(0, t.var_lines[v][0]), want);
        }
    }
}
