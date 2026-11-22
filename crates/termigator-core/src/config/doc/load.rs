//! PyYAML `safe_load` on libyaml's event parser: the Composer (one document, anchors
//! and aliases) and the SafeConstructor (YAML 1.1 types, merge keys, dict semantics).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use libyaml_safer::{Event, EventData, Mark, Parser, ScalarStyle};

use super::resolve::{self, NS, Tag};
use super::{Value, YamlError};

/// Deeper nesting is refused instead of overflowing a 2 MiB thread stack (real files
/// nest under 20).
const MAX_DEPTH: usize = 200;
/// Aliases are expanded into copies; this caps an alias bomb.
const MAX_NODES: usize = 1_000_000;

struct Node {
    kind: Kind,
    tag: Option<String>,
    mark: Mark,
}

enum Kind {
    Scalar { value: String, plain: bool },
    Seq(Vec<Rc<Node>>),
    Map(Vec<(Rc<Node>, Rc<Node>)>),
}

fn error(mark: &Mark, message: impl Into<String>) -> YamlError {
    YamlError::Construct {
        line: mark.line + 1,
        column: mark.column + 1,
        message: message.into(),
    }
}

pub(super) fn load(text: &str) -> Result<Value, YamlError> {
    let mut parser = Parser::new();
    parser.set_input(text.as_bytes());
    let mut composer = Composer {
        parser,
        anchors: HashMap::new(),
        open: HashSet::new(),
    };
    composer.next()?; // StreamStart
    let root = match composer.next()? {
        Event {
            data: EventData::StreamEnd,
            ..
        } => return Ok(Value::Null),
        Event {
            data: EventData::DocumentStart { .. },
            ..
        } => {
            let first = composer.next()?;
            let root = composer.compose(first, 0)?;
            composer.next()?; // DocumentEnd
            root
        }
        other => return Err(error(&other.start_mark, "expected the start of a document")),
    };
    let after = composer.next()?;
    if !matches!(after.data, EventData::StreamEnd) {
        return Err(error(
            &after.start_mark,
            "expected a single document in the stream, but found another document",
        ));
    }
    Constructor { budget: MAX_NODES }.value(&root)
}

struct Composer<'a> {
    parser: Parser<&'a [u8]>,
    anchors: HashMap<String, Rc<Node>>,
    /// Anchors of nodes still being composed: an alias to one would be recursive.
    open: HashSet<String>,
}

impl Composer<'_> {
    fn next(&mut self) -> Result<Event, YamlError> {
        self.parser
            .parse()
            .map_err(|e| YamlError::Syntax(e.to_string()))
    }

    fn open_anchor(&mut self, anchor: Option<&String>, mark: &Mark) -> Result<(), YamlError> {
        match anchor {
            Some(a) if self.anchors.contains_key(a) || !self.open.insert(a.clone()) => {
                Err(error(mark, format!("found duplicate anchor {a:?}")))
            }
            _ => Ok(()),
        }
    }

    fn compose(&mut self, event: Event, depth: usize) -> Result<Rc<Node>, YamlError> {
        let mark = event.start_mark;
        if depth > MAX_DEPTH {
            return Err(error(
                &mark,
                format!("nesting is deeper than {MAX_DEPTH} levels"),
            ));
        }
        let (anchor, tag, kind) = match event.data {
            EventData::Alias { anchor } => {
                return match self.anchors.get(&anchor) {
                    Some(node) => Ok(Rc::clone(node)),
                    None if self.open.contains(&anchor) => Err(error(
                        &mark,
                        format!("recursive alias {anchor:?} is not supported"),
                    )),
                    None => Err(error(&mark, format!("found undefined alias {anchor:?}"))),
                };
            }
            EventData::Scalar {
                anchor,
                tag,
                value,
                style,
                ..
            } => {
                self.open_anchor(anchor.as_ref(), &mark)?;
                (
                    anchor,
                    tag,
                    Kind::Scalar {
                        value,
                        plain: style == ScalarStyle::Plain,
                    },
                )
            }
            EventData::SequenceStart { anchor, tag, .. } => {
                self.open_anchor(anchor.as_ref(), &mark)?;
                let mut items = Vec::new();
                loop {
                    let ev = self.next()?;
                    if matches!(ev.data, EventData::SequenceEnd) {
                        break;
                    }
                    items.push(self.compose(ev, depth + 1)?);
                }
                (anchor, tag, Kind::Seq(items))
            }
            EventData::MappingStart { anchor, tag, .. } => {
                self.open_anchor(anchor.as_ref(), &mark)?;
                let mut pairs = Vec::new();
                loop {
                    let ev = self.next()?;
                    if matches!(ev.data, EventData::MappingEnd) {
                        break;
                    }
                    let key = self.compose(ev, depth + 1)?;
                    let value = self.next()?;
                    pairs.push((key, self.compose(value, depth + 1)?));
                }
                (anchor, tag, Kind::Map(pairs))
            }
            _ => return Err(error(&mark, "expected a node")),
        };
        let node = Rc::new(Node { kind, tag, mark });
        if let Some(a) = anchor {
            self.open.remove(&a);
            self.anchors.insert(a, Rc::clone(&node));
        }
        Ok(node)
    }
}

/// The resolved tag of a scalar node; `None` for collections and unknown tags.
fn scalar_tag(node: &Node) -> Option<Tag> {
    let Kind::Scalar { value, plain } = &node.kind else {
        return None;
    };
    match node.tag.as_deref() {
        // PyYAML resolves the non-specific `!` like a plain scalar, even when quoted.
        None if *plain => Some(resolve::resolve_plain(value)),
        Some("!") => Some(resolve::resolve_plain(value)),
        None => Some(Tag::Str),
        Some(uri) => Tag::from_uri(uri),
    }
}

fn kind_name(node: &Node) -> &'static str {
    match node.kind {
        Kind::Scalar { .. } => "scalar",
        Kind::Seq(_) => "sequence",
        Kind::Map(_) => "mapping",
    }
}

struct Constructor {
    budget: usize,
}

impl Constructor {
    fn spend(&mut self, n: usize, mark: &Mark) -> Result<(), YamlError> {
        self.budget = self.budget.checked_sub(n).ok_or_else(|| {
            error(
                mark,
                format!("document expands to more than {MAX_NODES} nodes"),
            )
        })?;
        Ok(())
    }

    fn value(&mut self, node: &Node) -> Result<Value, YamlError> {
        self.spend(1, &node.mark)?;
        let collection_tag_ok = |default: &str| match node.tag.as_deref() {
            None | Some("!") => Ok(()),
            Some(t) if t.strip_prefix(NS) == Some(default) => Ok(()),
            Some(t) => Err(error(
                &node.mark,
                format!(
                    "could not determine a constructor for the tag '{t}' on a {}",
                    kind_name(node)
                ),
            )),
        };
        match &node.kind {
            Kind::Scalar { value, .. } => {
                let tag = scalar_tag(node).ok_or_else(|| {
                    let t = node.tag.as_deref().unwrap_or_default();
                    error(
                        &node.mark,
                        format!("could not determine a constructor for the tag '{t}'"),
                    )
                })?;
                resolve::construct(tag, value).map_err(|m| error(&node.mark, m))
            }
            Kind::Seq(items) => {
                collection_tag_ok("seq")?;
                items
                    .iter()
                    .map(|n| self.value(n))
                    .collect::<Result<_, _>>()
                    .map(Value::Seq)
            }
            Kind::Map(_) => {
                collection_tag_ok("map")?;
                self.mapping(node)
            }
        }
    }

    /// `SafeConstructor.construct_mapping`: flatten merges, then fill a dict.
    fn mapping(&mut self, node: &Node) -> Result<Value, YamlError> {
        let mut out: Vec<(Value, Value)> = Vec::new();
        for (k, v) in self.flatten(node)? {
            // flatten_mapping retags `=` keys as plain strings.
            let key = match scalar_tag(&k) {
                Some(Tag::Value) => Value::Str("=".to_owned()),
                _ => self.value(&k)?,
            };
            if matches!(key, Value::Seq(_) | Value::Map(_)) {
                return Err(error(&k.mark, "found unhashable key"));
            }
            let value = self.value(&v)?;
            // A dict keeps the first key (and its position) and the last value.
            match out.iter_mut().find(|(existing, _)| existing.loose_eq(&key)) {
                Some(slot) => slot.1 = value,
                None => out.push((key, value)),
            }
        }
        Ok(Value::Map(out))
    }

    /// `SafeConstructor.flatten_mapping`: pairs merged in through `<<` come first.
    #[allow(clippy::type_complexity)]
    fn flatten(&mut self, node: &Node) -> Result<Vec<(Rc<Node>, Rc<Node>)>, YamlError> {
        let Kind::Map(pairs) = &node.kind else {
            return Err(error(
                &node.mark,
                format!("expected a mapping, but found {}", kind_name(node)),
            ));
        };
        let mut merged = Vec::new();
        let mut own = Vec::new();
        for (k, v) in pairs {
            if scalar_tag(k) != Some(Tag::Merge) {
                own.push((Rc::clone(k), Rc::clone(v)));
                continue;
            }
            match &v.kind {
                Kind::Map(_) => merged.extend(self.flatten(v)?),
                Kind::Seq(items) => {
                    let mut parts = Vec::new();
                    for item in items {
                        if !matches!(item.kind, Kind::Map(_)) {
                            return Err(error(
                                &item.mark,
                                format!(
                                    "expected a mapping for merging, but found {}",
                                    kind_name(item)
                                ),
                            ));
                        }
                        parts.push(self.flatten(item)?);
                    }
                    // Earlier mappings in the list win, so they go last.
                    merged.extend(parts.into_iter().rev().flatten());
                }
                Kind::Scalar { .. } => {
                    return Err(error(
                        &v.mark,
                        "expected a mapping or list of mappings for merging, but found scalar",
                    ));
                }
            }
            self.spend(merged.len(), &v.mark)?;
        }
        merged.extend(own);
        Ok(merged)
    }
}
