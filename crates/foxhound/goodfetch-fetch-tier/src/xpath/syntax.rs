//! The grammar of an `XPath` 1.0 expression (W3C, `XPath` 1.0, §2 to §4),
//! read from its tokens (`tokens`) into a checked tree: what a browser's
//! `document.evaluate` refuses with no namespace resolver, it refuses too
//! (a prefix, an unknown function or a wrong number of arguments), as it
//! refuses a variable, which nothing binds, and an expression that answers
//! a value where nodes are due (`count(//a)`, `"a" | //b`), since a
//! browser throws reading such a result as nodes.

use super::tokens::{Token, tokens};

/// How deep expressions may nest (parentheses, predicates, arguments and
/// unary minus), so neither reading nor evaluating one runs out of stack.
const DEEPEST: usize = 100;

/// A checked expression.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Expr {
    /// `a or b or …`, two or more.
    Or(Vec<Expr>),
    /// `a and b and …`, two or more.
    And(Vec<Expr>),
    /// `a = b != c …`, read from the left.
    Compare(Box<Expr>, Vec<(Comparison, Expr)>),
    /// `a + b * c …` at one precedence level, read from the left.
    Arithmetic(Box<Expr>, Vec<(Arithmetic, Expr)>),
    /// `-a`.
    Negate(Box<Expr>),
    /// `a | b | …`, two or more node-sets.
    Union(Vec<Expr>),
    /// A location path, or a filter expression's nodes followed by steps.
    Path(Path),
    /// A primary expression's nodes filtered by predicates.
    Filter(Box<Expr>, Vec<Expr>),
    /// A string literal.
    Literal(String),
    /// A number.
    Number(f64),
    /// A core library function called with its arguments.
    Call(Function, Vec<Expr>),
}

/// A location path: where it starts, then its steps.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Path {
    pub(super) start: Start,
    pub(super) steps: Vec<Step>,
}

/// Where a path starts.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Start {
    /// The document's root, `/…`.
    Root,
    /// The context node, a relative path.
    Context,
    /// The nodes a filter expression answers, `(…)/…`.
    Nodes(Box<Expr>),
}

/// One step: an axis, a node test and its predicates.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Step {
    pub(super) axis: Axis,
    pub(super) test: NodeTest,
    pub(super) predicates: Vec<Expr>,
}

/// The thirteen axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Namespace,
    Parent,
    Preceding,
    PrecedingSibling,
    Itself,
}

impl Axis {
    pub(super) fn named(name: &str) -> Option<Self> {
        Some(match name {
            "ancestor" => Self::Ancestor,
            "ancestor-or-self" => Self::AncestorOrSelf,
            "attribute" => Self::Attribute,
            "child" => Self::Child,
            "descendant" => Self::Descendant,
            "descendant-or-self" => Self::DescendantOrSelf,
            "following" => Self::Following,
            "following-sibling" => Self::FollowingSibling,
            "namespace" => Self::Namespace,
            "parent" => Self::Parent,
            "preceding" => Self::Preceding,
            "preceding-sibling" => Self::PrecedingSibling,
            "self" => Self::Itself,
            _ => return None,
        })
    }
}

/// What a step's node must be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NodeTest {
    /// A node of the axis's principal type with this local name and no
    /// namespace prefix.
    Name(String),
    /// Any node of the axis's principal type, `*`.
    Principal,
    /// `node()`.
    Node,
    /// `text()`.
    Text,
    /// `comment()`.
    Comment,
    /// `processing-instruction()`, with the target its literal names.
    Instruction(Option<String>),
}

/// An equality or relational operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

/// An additive or multiplicative operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
}

/// The core function library (§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Function {
    Last,
    Position,
    Count,
    Id,
    LocalName,
    NamespaceUri,
    Name,
    String,
    Concat,
    StartsWith,
    Contains,
    SubstringBefore,
    SubstringAfter,
    Substring,
    StringLength,
    NormalizeSpace,
    Translate,
    Boolean,
    Not,
    True,
    False,
    Lang,
    Number,
    Sum,
    Floor,
    Ceiling,
    Round,
}

/// Each core function by its name.
const FUNCTIONS: [(&str, Function); 27] = [
    ("last", Function::Last),
    ("position", Function::Position),
    ("count", Function::Count),
    ("id", Function::Id),
    ("local-name", Function::LocalName),
    ("namespace-uri", Function::NamespaceUri),
    ("name", Function::Name),
    ("string", Function::String),
    ("concat", Function::Concat),
    ("starts-with", Function::StartsWith),
    ("contains", Function::Contains),
    ("substring-before", Function::SubstringBefore),
    ("substring-after", Function::SubstringAfter),
    ("substring", Function::Substring),
    ("string-length", Function::StringLength),
    ("normalize-space", Function::NormalizeSpace),
    ("translate", Function::Translate),
    ("boolean", Function::Boolean),
    ("not", Function::Not),
    ("true", Function::True),
    ("false", Function::False),
    ("lang", Function::Lang),
    ("number", Function::Number),
    ("sum", Function::Sum),
    ("floor", Function::Floor),
    ("ceiling", Function::Ceiling),
    ("round", Function::Round),
];

impl Function {
    fn named(name: &str) -> Option<Self> {
        FUNCTIONS
            .iter()
            .find_map(|&(named, function)| (named == name).then_some(function))
    }

    /// Its name.
    fn name(self) -> &'static str {
        FUNCTIONS
            .iter()
            .find_map(|&(name, function)| (function == self).then_some(name))
            .unwrap_or_default()
    }

    /// The fewest and the most arguments it takes; `None`, no most.
    const fn arity(self) -> (usize, Option<usize>) {
        match self {
            Self::Last | Self::Position | Self::True | Self::False => (0, Some(0)),
            Self::LocalName
            | Self::NamespaceUri
            | Self::Name
            | Self::String
            | Self::StringLength
            | Self::NormalizeSpace
            | Self::Number => (0, Some(1)),
            Self::Count
            | Self::Id
            | Self::Boolean
            | Self::Not
            | Self::Lang
            | Self::Sum
            | Self::Floor
            | Self::Ceiling
            | Self::Round => (1, Some(1)),
            Self::StartsWith | Self::Contains | Self::SubstringBefore | Self::SubstringAfter => {
                (2, Some(2))
            }
            Self::Substring => (2, Some(3)),
            Self::Translate => (3, Some(3)),
            Self::Concat => (2, None),
        }
    }

    /// Whether each of its arguments must be a node-set.
    const fn takes_nodes(self) -> bool {
        matches!(
            self,
            Self::Count | Self::Sum | Self::LocalName | Self::NamespaceUri | Self::Name
        )
    }
}

/// Reads `text` into a checked expression that answers nodes.
///
/// # Errors
///
/// Why `text` is no such expression, in a clause.
pub(super) fn parse(text: &str) -> Result<Expr, String> {
    let tokens = tokens(text)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
    };
    let expr = parser.expression()?;
    if let Some(token) = parser.tokens.get(parser.at) {
        return Err(format!("it goes on past its end at {}", token.shown()));
    }
    if answers(&expr)? == Answer::Value {
        return Err("it answers a value, not nodes".to_owned());
    }
    Ok(expr)
}

/// A recursive-descent reader of tokens (§3.1 to §3.5).
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn eat(&mut self, token: &Token) -> bool {
        let eaten = self.peek() == Some(token);
        if eaten {
            self.at += 1;
        }
        eaten
    }

    fn expect(&mut self, token: &Token) -> Result<(), String> {
        if self.eat(token) {
            return Ok(());
        }
        Err(match self.peek() {
            Some(found) => format!("{} stands where {} is due", found.shown(), token.shown()),
            None => format!("it ends where {} is due", token.shown()),
        })
    }

    /// Enters one nesting level, refused past [`DEEPEST`].
    fn nest(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > DEEPEST {
            return Err(format!("it nests deeper than {DEEPEST} levels"));
        }
        Ok(())
    }

    fn expression(&mut self) -> Result<Expr, String> {
        self.nest()?;
        let expr = self.or();
        self.depth -= 1;
        expr
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut operands = vec![self.and()?];
        while self.eat(&Token::Or) {
            operands.push(self.and()?);
        }
        Ok(if operands.len() == 1 {
            operands.remove(0)
        } else {
            Expr::Or(operands)
        })
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut operands = vec![self.comparison()?];
        while self.eat(&Token::And) {
            operands.push(self.comparison()?);
        }
        Ok(if operands.len() == 1 {
            operands.remove(0)
        } else {
            Expr::And(operands)
        })
    }

    /// An equality expression of relational ones: `=` and `!=` bind more
    /// loosely than `<`, `<=`, `>` and `>=`.
    fn comparison(&mut self) -> Result<Expr, String> {
        let first = self.relational()?;
        let mut rest = Vec::new();
        while let Some(&Token::Compare(operator @ (Comparison::Equal | Comparison::NotEqual))) =
            self.peek()
        {
            self.at += 1;
            rest.push((operator, self.relational()?));
        }
        Ok(if rest.is_empty() {
            first
        } else {
            Expr::Compare(Box::new(first), rest)
        })
    }

    fn relational(&mut self) -> Result<Expr, String> {
        let first = self.additive()?;
        let mut rest = Vec::new();
        while let Some(&Token::Compare(
            operator @ (Comparison::Less
            | Comparison::LessOrEqual
            | Comparison::Greater
            | Comparison::GreaterOrEqual),
        )) = self.peek()
        {
            self.at += 1;
            rest.push((operator, self.additive()?));
        }
        Ok(if rest.is_empty() {
            first
        } else {
            Expr::Compare(Box::new(first), rest)
        })
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let first = self.multiplicative()?;
        let mut rest = Vec::new();
        loop {
            let operator = match self.peek() {
                Some(Token::Plus) => Arithmetic::Add,
                Some(Token::Minus) => Arithmetic::Subtract,
                _ => break,
            };
            self.at += 1;
            rest.push((operator, self.multiplicative()?));
        }
        Ok(if rest.is_empty() {
            first
        } else {
            Expr::Arithmetic(Box::new(first), rest)
        })
    }

    fn multiplicative(&mut self) -> Result<Expr, String> {
        let first = self.unary()?;
        let mut rest = Vec::new();
        while let Some(&Token::Multiplicative(operator)) = self.peek() {
            self.at += 1;
            rest.push((operator, self.unary()?));
        }
        Ok(if rest.is_empty() {
            first
        } else {
            Expr::Arithmetic(Box::new(first), rest)
        })
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Minus) {
            self.nest()?;
            let operand = self.unary();
            self.depth -= 1;
            return Ok(Expr::Negate(Box::new(operand?)));
        }
        self.union()
    }

    fn union(&mut self) -> Result<Expr, String> {
        let mut operands = vec![self.path()?];
        while self.eat(&Token::Pipe) {
            operands.push(self.path()?);
        }
        Ok(if operands.len() == 1 {
            operands.remove(0)
        } else {
            Expr::Union(operands)
        })
    }

    fn path(&mut self) -> Result<Expr, String> {
        let start = match self.peek() {
            Some(Token::Slash) => {
                self.at += 1;
                let mut steps = Vec::new();
                if self.peek().is_some_and(starts_step) {
                    self.relative(&mut steps)?;
                }
                return Ok(Expr::Path(Path {
                    start: Start::Root,
                    steps,
                }));
            }
            Some(Token::DoubleSlash) => {
                self.at += 1;
                let mut steps = vec![anywhere()];
                self.relative(&mut steps)?;
                return Ok(Expr::Path(Path {
                    start: Start::Root,
                    steps,
                }));
            }
            Some(
                Token::Variable(_)
                | Token::LeftParen
                | Token::Literal(_)
                | Token::Number(_)
                | Token::FunctionName { .. },
            ) => self.filter()?,
            Some(token) if starts_step(token) => {
                let mut steps = Vec::new();
                self.relative(&mut steps)?;
                return Ok(Expr::Path(Path {
                    start: Start::Context,
                    steps,
                }));
            }
            Some(token) => return Err(format!("{} stands where a path is due", token.shown())),
            None => return Err("it ends where a path is due".to_owned()),
        };
        let mut steps = Vec::new();
        match self.peek() {
            Some(Token::Slash) => self.at += 1,
            Some(Token::DoubleSlash) => {
                self.at += 1;
                steps.push(anywhere());
            }
            _ => return Ok(start),
        }
        self.relative(&mut steps)?;
        Ok(Expr::Path(Path {
            start: Start::Nodes(Box::new(start)),
            steps,
        }))
    }

    /// A relative location path's steps, pushed onto `steps`.
    fn relative(&mut self, steps: &mut Vec<Step>) -> Result<(), String> {
        steps.push(self.step()?);
        loop {
            match self.peek() {
                Some(Token::Slash) => self.at += 1,
                Some(Token::DoubleSlash) => {
                    self.at += 1;
                    steps.push(anywhere());
                }
                _ => return Ok(()),
            }
            steps.push(self.step()?);
        }
    }

    fn step(&mut self) -> Result<Step, String> {
        let axis = match self.peek() {
            Some(Token::Dot) => {
                self.at += 1;
                return Ok(Step {
                    axis: Axis::Itself,
                    test: NodeTest::Node,
                    predicates: Vec::new(),
                });
            }
            Some(Token::DotDot) => {
                self.at += 1;
                return Ok(Step {
                    axis: Axis::Parent,
                    test: NodeTest::Node,
                    predicates: Vec::new(),
                });
            }
            Some(&Token::AxisName(axis)) => {
                self.at += 1;
                self.expect(&Token::DoubleColon)?;
                axis
            }
            Some(Token::At) => {
                self.at += 1;
                Axis::Attribute
            }
            _ => Axis::Child,
        };
        let test = match self.bump() {
            Some(Token::NameTest {
                prefix: Some(prefix),
                ..
            }) => {
                return Err(format!(
                    "it names the namespace prefix `{prefix}`, which nothing binds"
                ));
            }
            Some(Token::NameTest {
                prefix: None,
                local,
            }) => local.map_or(NodeTest::Principal, NodeTest::Name),
            Some(Token::NodeType(test)) => {
                self.expect(&Token::LeftParen)?;
                let test = match (test, self.peek()) {
                    (NodeTest::Instruction(None), Some(Token::Literal(target))) => {
                        let target = target.clone();
                        self.at += 1;
                        NodeTest::Instruction(Some(target))
                    }
                    (test, _) => test,
                };
                self.expect(&Token::RightParen)?;
                test
            }
            Some(token) => {
                return Err(format!("{} stands where a node test is due", token.shown()));
            }
            None => return Err("it ends where a node test is due".to_owned()),
        };
        Ok(Step {
            axis,
            test,
            predicates: self.predicates()?,
        })
    }

    fn predicates(&mut self) -> Result<Vec<Expr>, String> {
        let mut predicates = Vec::new();
        while self.eat(&Token::LeftBracket) {
            predicates.push(self.expression()?);
            self.expect(&Token::RightBracket)?;
        }
        Ok(predicates)
    }

    fn filter(&mut self) -> Result<Expr, String> {
        let primary = match self.bump() {
            Some(Token::Variable(name)) => {
                return Err(format!(
                    "it names the variable `${name}`, which nothing binds"
                ));
            }
            Some(Token::LeftParen) => {
                let inner = self.expression()?;
                self.expect(&Token::RightParen)?;
                inner
            }
            Some(Token::Literal(literal)) => Expr::Literal(literal),
            Some(Token::Number(number)) => Expr::Number(number),
            Some(Token::FunctionName { prefix, local }) => {
                let function = prefix
                    .is_none()
                    .then(|| Function::named(&local))
                    .flatten()
                    .ok_or_else(|| {
                        format!("it calls `{local}()`, which XPath 1.0 has no function named")
                    })?;
                self.expect(&Token::LeftParen)?;
                let mut arguments = Vec::new();
                if !self.eat(&Token::RightParen) {
                    arguments.push(self.expression()?);
                    while self.eat(&Token::Comma) {
                        arguments.push(self.expression()?);
                    }
                    self.expect(&Token::RightParen)?;
                }
                let (fewest, most) = function.arity();
                if arguments.len() < fewest || most.is_some_and(|most| arguments.len() > most) {
                    return Err(format!(
                        "it calls `{local}()` with {} arguments",
                        arguments.len()
                    ));
                }
                Expr::Call(function, arguments)
            }
            Some(token) => return Err(format!("{} stands where a value is due", token.shown())),
            None => return Err("it ends where a value is due".to_owned()),
        };
        let predicates = self.predicates()?;
        Ok(if predicates.is_empty() {
            primary
        } else {
            Expr::Filter(Box::new(primary), predicates)
        })
    }
}

/// Whether `token` starts a step.
const fn starts_step(token: &Token) -> bool {
    matches!(
        token,
        Token::NameTest { .. }
            | Token::NodeType(_)
            | Token::AxisName(_)
            | Token::At
            | Token::Dot
            | Token::DotDot
    )
}

/// `//`'s step, `descendant-or-self::node()`.
const fn anywhere() -> Step {
    Step {
        axis: Axis::DescendantOrSelf,
        test: NodeTest::Node,
        predicates: Vec::new(),
    }
}

/// What an expression answers: nodes, or a value of another type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Nodes,
    Value,
}

/// What `expr` answers, refusing a node-set operation on a value, as a
/// browser does when it evaluates one.
fn answers(expr: &Expr) -> Result<Answer, String> {
    let nodes = |expr: &Expr, what: &str| match answers(expr)? {
        Answer::Nodes => Ok(()),
        Answer::Value => Err(format!("{what} takes nodes, and it is given a value")),
    };
    Ok(match expr {
        Expr::Or(operands) | Expr::And(operands) => {
            operands
                .iter()
                .try_for_each(|operand| answers(operand).map(drop))?;
            Answer::Value
        }
        Expr::Compare(first, rest) => {
            answers(first)?;
            rest.iter()
                .try_for_each(|(_, operand)| answers(operand).map(drop))?;
            Answer::Value
        }
        Expr::Arithmetic(first, rest) => {
            answers(first)?;
            rest.iter()
                .try_for_each(|(_, operand)| answers(operand).map(drop))?;
            Answer::Value
        }
        Expr::Negate(operand) => {
            answers(operand)?;
            Answer::Value
        }
        Expr::Union(operands) => {
            operands
                .iter()
                .try_for_each(|operand| nodes(operand, "`|`"))?;
            Answer::Nodes
        }
        Expr::Path(path) => {
            if let Start::Nodes(start) = &path.start {
                nodes(start, "a path")?;
            }
            path.steps
                .iter()
                .flat_map(|step| &step.predicates)
                .try_for_each(|predicate| answers(predicate).map(drop))?;
            Answer::Nodes
        }
        Expr::Filter(primary, predicates) => {
            nodes(primary, "a predicate")?;
            predicates
                .iter()
                .try_for_each(|predicate| answers(predicate).map(drop))?;
            Answer::Nodes
        }
        Expr::Literal(_) | Expr::Number(_) => Answer::Value,
        Expr::Call(function, arguments) => {
            for argument in arguments {
                if function.takes_nodes() {
                    nodes(argument, &format!("`{}()`", function.name()))?;
                } else {
                    answers(argument)?;
                }
            }
            if *function == Function::Id {
                Answer::Nodes
            } else {
                Answer::Value
            }
        }
    })
}
