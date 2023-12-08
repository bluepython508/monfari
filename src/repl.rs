use std::sync::{Arc, RwLock};

use eyre::{eyre, Result};
use itertools::Itertools;
use tracing::instrument;

use crate::{
    command::{self, AccountModification},
    repository::Repository,
    types::{
        Account, AccountType, Amount, Currency, Id, Physical, Transaction, TransactionInner,
        Virtual,
    },
};
use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, DefaultPrompt, DefaultPromptSegment, Emacs,
    Highlighter, KeyCode, KeyModifiers, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span,
    StyledText, Suggestion, ValidationResult, Validator,
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
    AccountShow {
        id: Id<Account>,
    },
    AccountModify(Id<Account>, Vec<AccountModification>),
    TransactionAdd {
        amount: Amount,
        inner: TransactionInner,
    },
}

struct Parser<'a> {
    iter: <&'a mut Vec<Token> as IntoIterator>::IntoIter,
    accounts: Vec<Account>,
}

impl<'a> Parser<'a> {
    fn parse(input: &str, accounts: Vec<Account>) -> (Vec<Token>, Result<Command, Completions>) {
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
            accounts,
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
            ("rename", &Self::account_rename),
            ("show", &Self::account_show),
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
        let id = self.account_id(None)?;
        Ok(Command::AccountModify(
            id,
            vec![AccountModification::Disable],
        ))
    }

    fn account_rename(&mut self) -> Result<Command, Completions> {
        let id = self.account_id(None)?;
        let name = self.string()?;
        Ok(Command::AccountModify(
            id,
            vec![AccountModification::UpdateName(name)],
        ))
    }

    fn account_show(&mut self) -> Result<Command, Completions> {
        let id = self.account_id(None)?;
        Ok(Command::AccountShow { id })
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
        self.expect("src")?;
        let src = self.string()?;
        self.expect("dst")?;
        let dst = self.account_phys()?;
        self.expect("dst-virt")?;
        let dst_virt = self.account_virt()?;
        Ok(TransactionInner::Received { src, dst, dst_virt })
    }

    fn transaction_paid(&mut self) -> Result<TransactionInner, Completions> {
        self.expect("dst")?;
        let dst = self.string()?;
        self.expect("src")?;
        let src = self.account_phys()?;
        self.expect("src-virt")?;
        let src_virt = self.account_virt()?;
        Ok(TransactionInner::Paid { src, dst, src_virt })
    }

    fn transaction_move_phys(&mut self) -> Result<TransactionInner, Completions> {
        self.expect("dst")?;
        let dst = self.account_phys()?;
        self.expect("src")?;
        let src = self.account_phys()?;
        Ok(TransactionInner::MovePhys { src, dst })
    }

    fn transaction_move_virt(&mut self) -> Result<TransactionInner, Completions> {
        self.expect("dst")?;
        let dst = self.account_virt()?;
        self.expect("src")?;
        let src = self.account_virt()?;
        Ok(TransactionInner::MoveVirt { src, dst })
    }

    fn transaction_convert(&mut self) -> Result<TransactionInner, Completions> {
        self.expect("into")?;
        let new_amount = self.amount()?;
        self.expect("account")?;
        let acc = self.account_phys()?;
        self.expect("virtual")?;
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
        self.token(None, |_, s| {
            Some((TokenType::String, s.trim_matches('"').to_owned()))
        })
    }

    fn account_id(
        &mut self,
        account_type: Option<AccountType>,
    ) -> Result<Id<Account>, Completions> {
        self.token(
            Some(
                self.accounts
                    .iter()
                    .filter(|x| x.enabled)
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
                        this.accounts
                            .iter()
                            .find(|x| x.id == s)
                            .is_some_and(|acc| account_type.map_or(true, |typ| acc.typ == typ))
                    })?,
                ))
            },
        )
    }

    fn account_phys(&mut self) -> Result<Id<Account<Physical>>, Completions> {
        self.account_id(Some(AccountType::Physical))
            .map(|x| x.unerase())
    }
    fn account_virt(&mut self) -> Result<Id<Account<Virtual>>, Completions> {
        self.account_id(Some(AccountType::Virtual))
            .map(|x| x.unerase())
    }

    fn expect(&mut self, x: &'static str) -> Result<(), Completions> {
        self.token(Some([x.to_string()].into_iter().collect()), |_, tok| {
            (tok == x).then_some((TokenType::Command, ()))
        })
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
struct ReedlineCmd(Arc<RwLock<Vec<Account>>>);
impl ReedlineCmd {
    fn parse(&self, line: &str) -> (Vec<Token>, Result<Command, Completions>) {
        Parser::parse(
            line,
            self.0
                .read()
                .unwrap()
                .clone(),
        )
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

pub async fn repl(mut repo: Repository) -> Result<Repository> {
    let custom = ReedlineCmd(Arc::new(RwLock::new(repo.accounts().await?)));
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
                if let Err(e) = run_command(&mut repo, &custom, line).await {
                    eprintln!("{e}");
                }
            }
            Signal::CtrlC => {}
            Signal::CtrlD => break,
        }
    }
    Ok(repo)
}

pub async fn command(mut repo: Repository, cmd: String) -> Result<Repository> {
    let custom = ReedlineCmd(Arc::new(RwLock::new(repo.accounts().await?)));
    run_command(&mut repo, &custom, cmd).await?;
    Ok(repo)
}

#[allow(clippy::await_holding_lock)]
async fn run_command(repo: &mut Repository, custom: &ReedlineCmd, cmd: String) -> Result<()> {
    let cmd = custom
        .parse(&cmd)
        .1
        .map_err(|_| eyre!("Invalid Command: {}", cmd))?;
    match cmd {
        Command::AccountsList => accounts_list(repo).await?,
        Command::AccountCreate { typ, name } => account_create(repo, typ, name).await?,
        Command::AccountShow { id } => account_show(repo, id).await?,
        Command::AccountModify(id, mods) => account_modify(repo, id, mods).await?,
        Command::TransactionAdd { amount, inner } => transaction(repo, amount, inner).await?,
    };
    *custom.0.write().unwrap() = repo.accounts().await?;
    Ok(())
}

#[instrument]
async fn transaction(repo: &mut Repository, amount: Amount, inner: TransactionInner) -> Result<()> {
    let notes = edit::edit("# Notes")?
        .lines()
        .filter(|x| !x.starts_with('#'))
        .collect();
    let id = Id::generate();
    repo.run_command(command::Command::AddTransaction(Transaction {
        id,
        notes,
        amount,
        inner,
    }))
    .await?;
    println!("Added transaction {}", id);
    Ok(())
}

#[instrument]
async fn account_modify(
    repo: &mut Repository,
    id: Id<Account>,
    mods: Vec<AccountModification>,
) -> Result<()> {
    repo.run_command(command::Command::UpdateAccount(id, mods))
        .await?;
    Ok(())
}

#[instrument]
async fn account_create(repo: &mut Repository, typ: AccountType, name: String) -> Result<()> {
    let notes = edit::edit("# Notes")?
        .lines()
        .filter(|x| !x.starts_with('#'))
        .collect();
    let id = Id::generate();
    repo.run_command(command::Command::CreateAccount(Account {
        id,
        name: name.clone(),
        notes,
        typ,
        current: Default::default(),
        enabled: true,
    }))
    .await?;
    println!("Created account \"{}\" ({})", name, id);
    Ok(())
}

#[instrument]
async fn accounts_list(repo: &Repository) -> Result<()> {
    use comfy_table::*;
    let mut table = Table::new();
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["ID", "Name", "Type", "Enabled", "Contents"]);
    table
        .column_mut(0)
        .expect("Column 0 exists")
        .set_delimiter('-');
    for account in repo.accounts().await? {
        let Account {
            id,
            name,
            typ,
            current,
            enabled,
            ..
        } = account;
        table.add_row(vec![
            id.to_string(),
            name,
            typ.to_string(),
            enabled.to_string(),
            current.to_string(),
        ]);
    }
    println!("{table}");
    Ok(())
}

async fn account_show(repo: &Repository, account: Id<Account>) -> Result<()> {
    let Account {
        id,
        name,
        typ,
        current,
        enabled: _,
        notes: _,
    } = repo.account(account).await?;
    let transactions = repo.transactions(id).await?;
    println!("{name} ({typ}: {id})");
    println!("{current}");
    use comfy_table::*;
    let mut table = Table::new();
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["Amount", "Description", "Notes"]);
    for transaction in transactions {
        let moved = |src, dst| async move {
            let (direction, other) = if src == account {
                ("into", dst)
            } else {
                ("from", src)
            };
            let name = repo.account(other).await?.name;
            Ok::<_, eyre::Report>(format!("Moved {direction} \"{name}\""))
        };
        let Transaction {
            id: _,
            notes,
            amount,
            inner,
        } = transaction;
        let desc = match inner {
            TransactionInner::Received { src, .. } => format!("Received from {src}"),
            TransactionInner::Paid { dst, .. } => format!("Paid to {dst}"),
            TransactionInner::MovePhys { src, dst } => moved(src.erase(), dst.erase()).await?,
            TransactionInner::MoveVirt { src, dst } => moved(src.erase(), dst.erase()).await?,
            TransactionInner::Convert { new_amount, .. } => format!("Converted into {new_amount}"),
        };
        table.add_row(vec![amount.to_string(), desc, notes]);
    }
    println!("{table}");
    Ok(())
}
