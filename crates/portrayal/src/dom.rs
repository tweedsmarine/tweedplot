//! A small XML tree, for the catalogue files (a few MB at most).

use quick_xml::events::Event;

#[derive(Debug, Default, Clone)]
pub struct Node {
    /// Element name without its namespace prefix.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// Trimmed text of a child, if present.
    pub fn text_of(&self, name: &str) -> Option<String> {
        self.child(name).map(|c| c.text.trim().to_string())
    }
}

fn local(name: impl AsRef<[u8]>) -> String {
    let s = String::from_utf8_lossy(name.as_ref());
    s.rsplit(':').next().unwrap_or("").to_string()
}

fn element(e: &quick_xml::events::BytesStart) -> Node {
    let attrs = e
        .attributes()
        .flatten()
        .map(|a| {
            let v: &str = &a.value;
            (local(a.key.into_inner()), v.to_string())
        })
        .collect();
    Node { name: local(e.name().into_inner()), attrs, ..Default::default() }
}

/// Parse a document into its root element.
pub fn parse(text: &str) -> Result<Node, String> {
    let mut r = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Node> = vec![Node::default()];
    loop {
        match r.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => stack.push(element(&e)),
            Event::Empty(e) => {
                let n = element(&e);
                stack.last_mut().unwrap().children.push(n);
            }
            Event::Text(t) => {
                let s: &str = &t;
                stack.last_mut().unwrap().text.push_str(s);
            }
            Event::CData(t) => {
                let s: &str = &t;
                stack.last_mut().unwrap().text.push_str(s);
            }
            Event::GeneralRef(g) => {
                let s: &str = &g;
                let c = match s {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                };
                stack.last_mut().unwrap().text.push_str(c);
            }
            Event::End(_) => {
                let n = stack.pop().unwrap();
                match stack.last_mut() {
                    Some(p) => p.children.push(n),
                    None => return Err("unbalanced".into()),
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut root = stack.pop().ok_or("empty")?;
    root.children.pop().ok_or_else(|| "no root element".to_string())
}
