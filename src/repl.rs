use std::sync::{Arc, RwLock};

use eyre::Result;
use itertools::Itertools;

use crate::{
    command::{AccountModification, self},
    repository::Repository,
    types::{Account, AccountType, Amount, Currency, Id, Physical, TransactionInner, Virtual, Transaction},
};
use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, DefaultPrompt, DefaultPromptSegment, Emacs,
    Highlighter, KeyCode, KeyModifiers, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span,
    StyledText, Suggestion, Validator, ValidationResult,
};

use nu_ansi_term::Color;

#[derive(Default, Debug, Clone)]
struct Completions(Vec<Suggestion>);

impl Completions {
    fn set_span(&mut self, span: Span) {
        for suggestion in self.0.iter_mut() {
            suggestion.span = span;
        }
    }
}

impl FromIterator<String> for Completions {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        iter.into_iter().map(|x| (x, None)).collect()
    }
}

impl FromIterator<(String, Option<String>)> for Completions {
    fn from_iter<T: IntoIterator<Item = (String, Option<String>)>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|(value, description)| Suggestion {
                    span: Span::new(0, 0),
                    value,
                    description,
                    extra: None,
                    append_whitespace: true,
                })
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenType {
    Command,
    String,
    Id,
    Amount,
    Invalid,
    Whitespace,
}

#[derive(Clone, Debug)]
struct Token {
    bounds: (usize, usize),
    str: String,
    typ: TokenType,
    completions: Completions,
}

#[derive(Debug)]
enum Command {
    AccountsList,
    AccountCreate {
        typ: AccountType,
        name: String,
    },
    AccountModify(Id<Account>, Vec<AccountModification>),
    TransactionAdd {
        amount: Amount,
        inner: TransactionInner,
    },
}

struct Parser<'a> {
    iter: <&'a mut Vec<Token> as IntoIterator>::IntoIter,
    repo: &'a Repository,
}

impl<'a> Parser<'a> {
    fn parse(input: &str, repo: &Repository) -> (Vec<Token>, Result<Command, Completions>) {
        let mut tokens = input
            .chars()
            .enumerate()
            .group_by({
                let mut in_string = false;
                move |&(_, c)| {
                    if c == '"' {
                        in_string = !in_string;
                        2
                    } else if in_string {
                        2
                    } else if c.is_whitespace() {
                        1
                    } else {
                        0
                    }
                }
            })
            .into_iter()
            .map(|(_, chars)| {
                let mut bounds = (usize::MAX, 0);
                let s = chars
                    .map(|(idx, c)| {
                        bounds.0 = usize::min(bounds.0, idx);
                        bounds.1 = usize::max(bounds.1, idx);
                        c
                    })
                    .collect::<String>();
                (bounds, s)
            })
            .map(|(bounds, str)| Token {
                typ: if str.chars().all(char::is_whitespace) {
                    TokenType::Whitespace
                } else {
                    TokenType::Invalid
                },
                completions: Completions::default(),
                bounds,
                str,
            })
            .collect::<Vec<_>>();
        let mut this = Parser {
            repo,
            iter: tokens.iter_mut(),
        };
        let mut res = this.run();
        for tok in &mut tokens {
            tok.completions
                .set_span(Span::new(tok.bounds.0, tok.bounds.1 + 1))
        }
        if let Err(completions) = &mut res {
            let end = tokens.last().map(|x| x.bounds.1 + 1).unwrap_or_default();
            completions.set_span(Span::new(end, end));
        }
        (tokens, res)
    }

    fn run(&mut self) -> Result<Command, Completions> {
        let value = self.dispatch(&[
            ("account", &Self::account),
            ("transaction", &Self::transaction),
        ])?;
        Ok(value)
    }

    fn account(&mut self) -> Result<Command, Completions> {
        self.dispatch(&[
            ("list", &|_| Ok(Command::AccountsList)),
            ("create", &Self::account_create),
            ("disable", &Self::account_disable),
        ])
    }

    fn account_create(&mut self) -> Result<Command, Completions> {
        let typ = self.dispatch(&[
            ("physical", &|_| Ok(AccountType::Physical)),
            ("virtual", &|_| Ok(AccountType::Virtual)),
        ])?;
        let name = self.string()?;
        Ok(Command::AccountCreate { typ, name })
    }

    fn account_disable(&mut self) -> Result<Command, Completions> {
        Ok(Command::AccountModify(
            self.account_id(None)?,
            vec![AccountModification::Disable],
        ))
    }

    fn transaction(&mut self) -> Result<Command, Completions> {
        let amount = self.amount()?;
        let inner = self.dispatch(&[
            ("received", &Self::transaction_received),
            ("paid", &Self::transaction_paid),
            ("move-phys", &Self::transaction_move_phys),
            ("move-virt", &Self::transaction_move_virt),
            ("convert", &Self::transaction_convert),
        ])?;
        Ok(Command::TransactionAdd { amount, inner })
    }

    fn transaction_received(&mut self) -> Result<TransactionInner, Completions> {
        self.oneof(&["src"])?;
        let src = self.string()?;
        self.oneof(&["dst"])?;
        let dst = self.account_phys()?;
        self.oneof(&["dst-virt"])?;
        let dst_virt = self.account_virt()?;
        Ok(TransactionInner::Received { src, dst, dst_virt })
    }

    fn transaction_paid(&mut self) -> Result<TransactionInner, Completions> {
        self.oneof(&["dst"])?;
        let dst = self.string()?;
        self.oneof(&["src"])?;
        let src = self.account_phys()?;
        self.oneof(&["src-virt"])?;
        let src_virt = self.account_virt()?;
        Ok(TransactionInner::Paid { src, dst, src_virt })
    }

    fn transaction_move_phys(&mut self) -> Result<TransactionInner, Completions> {
        self.oneof(&["dst"])?;
        let dst = self.account_phys()?;
        self.oneof(&["src"])?;
        let src = self.account_phys()?;
        self.oneof(&["with-fees"])?;
        let fees = self.token(None, |_, s| {
            Some((TokenType::Amount, s.parse::<i32>().ok()?))
        })?;
        Ok(TransactionInner::MovePhys { src, dst, fees })
    }

    fn transaction_move_virt(&mut self) -> Result<TransactionInner, Completions> {
        self.oneof(&["dst"])?;
        let dst = self.account_virt()?;
        self.oneof(&["src"])?;
        let src = self.account_virt()?;
        Ok(TransactionInner::MoveVirt { src, dst })
    }

    fn transaction_convert(&mut self) -> Result<TransactionInner, Completions> {
        self.oneof(&["into"])?;
        let new_amount = self.amount()?;
        self.oneof(&["account"])?;
        let acc = self.account_phys()?;
        self.oneof(&["virtual"])?;
        let acc_virt = self.account_virt()?;
        Ok(TransactionInner::Convert {
            acc,
            acc_virt,
            new_amount,
        })
    }

    fn amount(&mut self) -> Result<Amount, Completions> {
        let amount = self.token(None, |_, tok| {
            Some((TokenType::Amount, Amount::parse_num(tok)?))
        })?;
        let currency = self.token(
            Some(
                [Currency::EUR, Currency::GBP, Currency::USD]
                    .into_iter()
                    .map(|x| x.to_string())
                    .collect(),
            ),
            |_, tok| Some((TokenType::Amount, tok.parse().ok()?)),
        )?;
        Ok(Amount(amount, currency))
    }

    fn string(&mut self) -> Result<String, Completions> {
        self.token(None, |_, s| Some((TokenType::String, s.trim_matches('"').to_owned())))
    }

    fn account_id(
        &mut self,
        account_type: Option<AccountType>,
    ) -> Result<Id<Account>, Completions> {
        self.token(
            Some(
                self.repo
                    .list::<Account>()
                    .into_iter()
                    .flat_map(|x| x.into_iter())
                    .filter_map(|x| self.repo.get(x).ok())
                    .filter(|x| account_type.map_or(true, |typ| x.typ == typ))
                    .map(|x| {
                        (
                            x.id.to_string(),
                            Some(format!("{} ({})", x.name, x.current)),
                        )
                    })
                    .collect(),
            ),
            |this, tok| {
                Some((
                    TokenType::Id,
                    tok.parse().ok().filter(|&s| {
                        this.repo
                            .get::<Account>(s)
                            .is_ok_and(|acc| account_type.map_or(false, |typ| acc.typ == typ))
                    })?,
                ))
            },
        )
    }

    fn account_phys(&mut self) -> Result<Id<Account<Physical>>, Completions> {
        self.account_id(Some(AccountType::Physical))
            .map(|x| x.erase().unerase())
    }
    fn account_virt(&mut self) -> Result<Id<Account<Virtual>>, Completions> {
        self.account_id(Some(AccountType::Virtual))
            .map(|x| x.erase().unerase())
    }

    fn oneof<'b>(&mut self, of: &'b [&'b str]) -> Result<&'b str, Completions> {
        self.token(
            Some(of.iter().copied().map(|x| x.to_owned()).collect()),
            |_, tok| {
                of.iter()
                    .find(|&&c| c == tok)
                    .map(|&tok| (TokenType::Command, tok))
            },
        )
    }

    #[allow(clippy::type_complexity)]
    fn dispatch<T>(
        &mut self,
        args: &[(&'static str, &dyn Fn(&mut Self) -> Result<T, Completions>)],
    ) -> Result<T, Completions> {
        self.token(
            Some(args.iter().map(|(key, _)| (*key).to_owned()).collect()),
            |this, tok| {
                args.iter()
                    .find(|(key, _)| key == &tok)
                    .map(|(_, f)| (TokenType::Command, f(this)))
            },
        )?
    }

    fn token<T>(
        &mut self,
        completions: Option<Completions>,
        f: impl FnOnce(&mut Self, &str) -> Option<(TokenType, T)>,
    ) -> Result<T, Completions> {
        let completions = completions.unwrap_or_default();
        let tok = self
            .iter
            .find(|x| x.typ != TokenType::Whitespace)
            .ok_or_else(|| completions.clone())?;
        tok.completions = completions;
        if let Some((typ, val)) = f(self, &tok.str) {
            tok.typ = typ;
            Ok(val)
        } else {
            Err(tok.completions.clone())
        }
    }
}

#[derive(Clone)]
struct ReedlineCmd(Arc<RwLock<Repository>>);
impl ReedlineCmd {
    fn parse(&self, line: &str) -> (Vec<Token>, Result<Command, Completions>) {
        Parser::parse(line, &self.0.read().unwrap())
    }
}
impl Completer for ReedlineCmd {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<reedline::Suggestion> {
        let (tokens, res) = self.parse(line);
        let token = tokens
            .into_iter()
            .find(|x| x.bounds.0 <= pos && x.bounds.1 + 1 >= pos)
            .filter(|x| x.typ != TokenType::Whitespace);
        let prefix = token.as_ref().map(|x| x.str.clone());
        token
            .map(|x| x.completions)
            .unwrap_or_else(|| res.err().unwrap_or_default())
            .0
            .into_iter()
            .filter(|x| {
                prefix
                    .as_ref()
                    .map_or(true, |prefix| x.value.starts_with(prefix))
            })
            .collect()
    }
}

impl Highlighter for ReedlineCmd {
    fn highlight(&self, line: &str, _: usize) -> reedline::StyledText {
        let tokens = self.parse(line).0;
        StyledText {
            buffer: tokens
                .into_iter()
                .map(|Token { str, typ, .. }| {
                    (
                        match typ {
                            TokenType::Command => Color::Blue.dimmed(),
                            TokenType::String => Color::LightGreen.normal(),
                            TokenType::Id => Color::Green.dimmed(),
                            TokenType::Amount => Color::LightBlue.normal(),
                            TokenType::Invalid => Color::Red.normal(),
                            TokenType::Whitespace => Default::default(),
                        },
                        str,
                    )
                })
                .collect(),
        }
    }
}

impl Validator for ReedlineCmd {
    fn validate(&self, line: &str) -> ValidationResult {
        if self.parse(line).1.is_ok() {
            ValidationResult::Complete
        } else {
            ValidationResult::Incomplete
        }
    }
}


pub fn repl(repo: Repository) -> Result<()> {
    let custom = ReedlineCmd(Arc::new(RwLock::new(repo)));
    let completion_menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );

    let edit_mode = Box::new(Emacs::new(keybindings));

    let mut line_editor = Reedline::create()
        .with_completer(Box::new(custom.clone()))
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_quick_completions(true)
        .with_partial_completions(true)
        .with_edit_mode(edit_mode)
        .with_highlighter(Box::new(custom.clone()))
        .with_validator(Box::new(custom.clone()));
    let prompt = DefaultPrompt::new(DefaultPromptSegment::Empty, DefaultPromptSegment::Empty);
    loop {
        match line_editor.read_line(&prompt)? {
            Signal::Success(line) => {
                let Ok(cmd) = custom.parse(&line).1 else { println!("Invalid command"); continue };
                let repo = &mut *custom.0.write().unwrap();
                match cmd {
                    Command::AccountsList => accounts_list(repo)?,
                    Command::AccountCreate { typ, name } => account_create(repo, typ, name)?,
                    Command::AccountModify(id, mods) => account_modify(repo, id, mods)?,
                    Command::TransactionAdd { amount, inner } => transaction(repo, amount, inner)?,
                }
            }
            Signal::CtrlD => break Ok(()),
            Signal::CtrlC => {}
        }
    }
}

fn transaction(repo: &mut Repository, amount: Amount, inner: TransactionInner) -> Result<()> {
    let notes = edit::edit("# Notes")?.lines().filter(|x| !x.starts_with('#')).collect();
    let id = Id::generate();
    repo.run_command(command::Command::AddTransaction(Transaction {
        id, notes, amount, inner
    }))?;
    println!("Added transaction {}", id);
    Ok(())
}

fn account_modify(repo: &mut Repository, id: Id<Account>, mods: Vec<AccountModification>) -> Result<()> {
    repo.run_command(command::Command::UpdateAccount(id, mods))?;
    Ok(())
}

fn account_create(repo: &mut Repository, typ: AccountType, name: String) -> Result<()> {
    let notes = edit::edit("# Notes")?.lines().filter(|x| !x.starts_with('#')).collect();
    let id = Id::generate();
    repo.run_command(command::Command::CreateAccount(Account {
        id,
        name: name.clone(),
        notes,
        typ,
        current: Default::default(),
        enabled: true,
    }))?;
    println!("Created account \"{}\" ({})", name, id);
    Ok(())
}

fn accounts_list(repo: &Repository) -> Result<()> {
    for account in repo.list::<Account>()? {
        let Account { id, name, typ, current, .. }= repo.get(account)?;
        println!("  {id} (\"{name}\"): {typ} {current}");
    }
    Ok(())
}