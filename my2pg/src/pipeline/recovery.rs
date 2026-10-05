//! Recover only observed, rollback-confirmed row failures from one immutable batch.
use crate::{
    config::RowErrorPolicy,
    model::{Diagnostic, EncodedBatch, Severity, TablePlan},
    postgres::{self, CopyStage, FailureKind, TargetConnection, TargetError},
    report::RunArtifacts,
};
use std::{collections::BTreeSet, io, ops::Range, sync::Arc, time::Duration};

/// Compatibility facade sharing the actor-owned global cap.
pub struct RejectBudget(Arc<crate::report::artifact_io::OwnedRejectBudget>);
impl RejectBudget {
    pub fn new(limit: u64) -> Self {
        Self(crate::report::artifact_io::OwnedRejectBudget::new(limit))
    }
    pub fn used(&self) -> u64 {
        self.0.used()
    }
    pub fn reserve(&self) -> Option<RejectTicket> {
        self.0.reserve().map(RejectTicket)
    }
}
pub struct RejectTicket(pub(super) crate::report::artifact_io::OwnedRejectTicket);
impl RejectTicket {
    pub fn commit(mut self) {
        self.0.commit();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryProgress {
    pub committed_rows: u64,
    pub rejected_rows: u64,
    pub active_rows: u64,
    pub pending_rows: u64,
}
pub struct RecoveryContext<'a> {
    pub policy: RowErrorPolicy,
    pub artifacts: &'a RunArtifacts,
    pub reject_budget: &'a RejectBudget,
    /// Publish cumulative ACK/durable-reject counts before any further await.
    pub observer: &'a mut (dyn FnMut(RecoveryProgress) -> io::Result<()> + Send),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryFailure {
    Database,
    RejectIo,
    RejectLimit,
    ObserverIo,
    InvalidBatch,
    RetryExhausted,
}
#[derive(Debug, thiserror::Error)]
#[error("COPY recovery stopped: {kind:?}")]
pub struct RecoveryError {
    pub kind: RecoveryFailure,
    pub progress: RecoveryProgress,
    pub target: Option<TargetError>,
}
fn error(
    kind: RecoveryFailure,
    progress: RecoveryProgress,
    target: Option<TargetError>,
) -> RecoveryError {
    RecoveryError {
        kind,
        progress,
        target,
    }
}
fn safety_error(message: &'static str) -> TargetError {
    TargetError {
        stage: CopyStage::Idle,
        kind: FailureKind::Configuration,
        sqlstate: None,
        message,
        cause: None,
    }
}
fn catalog_error(source: tokio_postgres::Error) -> TargetError {
    TargetError {
        stage: CopyStage::Idle,
        kind: FailureKind::Operational,
        sqlstate: source.code().map(|code| code.code().to_owned()),
        message: "COPY recovery safety catalog query failed",
        cause: None,
    }
}

/// Inspect actual target behavior. A SQLSTATE alone cannot certify replay safety.
pub async fn check_recovery_safety(
    target: &TargetConnection,
    table: &TablePlan,
) -> Result<(), TargetError> {
    let relation = target.client.query_opt(
        "SELECT c.oid, EXISTS(SELECT 1 FROM pg_catalog.pg_am am WHERE am.oid=c.relam AND am.oid<16384 AND am.amname='heap' AND am.amtype='t') FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind='r'",
        &[&table.target_schema, &table.target_name],
    ).await.map_err(catalog_error)?.ok_or_else(|| safety_error("reject recovery requires an existing ordinary target table"))?;
    let oid: u32 = relation.get(0);
    let inherited: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_inherits WHERE inhparent=$1 OR inhrelid=$1)",
            &[&oid],
        )
        .await
        .map_err(catalog_error)?
        .get(0);
    if inherited {
        return Err(safety_error(
            "reject recovery does not support inherited or partition-member target tables",
        ));
    }
    if !relation.get::<_, bool>(1) {
        return Err(safety_error(
            "reject recovery requires the built-in heap table access method",
        ));
    }
    let unsafe_behavior: bool = target.client.query_one(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_trigger WHERE tgrelid=$1 AND NOT tgisinternal AND tgenabled<>'D') OR EXISTS(SELECT 1 FROM pg_catalog.pg_index i, unnest(i.indclass) oc WHERE i.indrelid=$1 AND oc>=16384)",
        &[&oid],
    ).await.map_err(catalog_error)?.get(0);
    if unsafe_behavior {
        return Err(safety_error(
            "reject recovery does not support enabled user triggers or custom index operator classes",
        ));
    }
    let attrs = target.client.query(
        "SELECT a.attname, (t.oid<16384 AND t.typnamespace='pg_catalog'::regnamespace AND t.typtype='b') OR (t.typtype='e' AND t.typinput='enum_in'::regproc) OR (t.typinput='array_in'::regproc AND ((e.oid<16384 AND e.typnamespace='pg_catalog'::regnamespace AND e.typtype='b') OR (e.typtype='e' AND e.typinput='enum_in'::regproc))), a.attgenerated<>'', pg_catalog.pg_get_expr(d.adbin,d.adrelid), a.attidentity<>'' FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid=a.atttypid LEFT JOIN pg_catalog.pg_type e ON e.oid=t.typelem LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped",
        &[&oid],
    ).await.map_err(catalog_error)?;
    let names: BTreeSet<String> = attrs.iter().map(|row| row.get(0)).collect();
    let builtin_text_arrays = target.client.query(
        "SELECT attname FROM pg_catalog.pg_attribute WHERE attrelid=$1 AND atttypid='pg_catalog.text[]'::regtype AND attnum>0 AND NOT attisdropped", &[&oid],
    ).await.map_err(catalog_error)?.into_iter().map(|row| row.get::<_,String>(0)).collect::<BTreeSet<_>>();
    for attr in &attrs {
        if !attr.get::<_, Option<bool>>(1).unwrap_or(false) {
            return Err(safety_error(
                "reject recovery does not support custom input types or domains",
            ));
        }
        let name: String = attr.get(0);
        let generated: bool = attr.get(2);
        let expression: Option<String> = attr.get(3);
        let copied = table
            .columns
            .iter()
            .any(|column| column.copy && column.target_name == name);
        if attr.get::<_, bool>(4) && !copied {
            return Err(safety_error(
                "reject recovery does not support omitted identity generation",
            ));
        }
        if (generated || !copied)
            && expression.is_some_and(|expression| !safe_expression(&expression, &names))
        {
            return Err(safety_error(
                "reject recovery requires a reviewed generated/default expression",
            ));
        }
    }
    // Catalog dependencies catch custom operators that deparse with an ordinary symbol.
    let custom_dependency: bool = target.client.query_one(
        "WITH objects AS (SELECT 'pg_constraint'::regclass AS classid, oid FROM pg_catalog.pg_constraint WHERE conrelid=$1 UNION ALL SELECT 'pg_attrdef'::regclass, oid FROM pg_catalog.pg_attrdef WHERE adrelid=$1 UNION ALL SELECT 'pg_class'::regclass, indexrelid FROM pg_catalog.pg_index WHERE indrelid=$1) SELECT EXISTS(SELECT 1 FROM objects o JOIN pg_catalog.pg_depend d ON d.classid=o.classid AND d.objid=o.oid WHERE d.refobjid>=16384 AND (d.refclassid IN ('pg_proc'::regclass,'pg_operator'::regclass,'pg_opclass'::regclass,'pg_opfamily'::regclass) OR (d.refclassid='pg_type'::regclass AND EXISTS(SELECT 1 FROM pg_catalog.pg_type t WHERE t.oid=d.refobjid AND t.typtype='d'))))",
        &[&oid],
    ).await.map_err(catalog_error)?.get(0);
    if custom_dependency {
        return Err(safety_error(
            "reject recovery does not support custom constraint/index/default dependencies",
        ));
    }
    let expressions = target.client.query(
        "SELECT pg_catalog.pg_get_expr(conbin,conrelid) AS expression, conname::text AS name FROM pg_catalog.pg_constraint WHERE conrelid=$1 AND contype='c' UNION ALL SELECT pg_catalog.pg_get_expr(indexprs,indrelid), NULL::text FROM pg_catalog.pg_index WHERE indrelid=$1 AND indexprs IS NOT NULL UNION ALL SELECT pg_catalog.pg_get_expr(indpred,indrelid), NULL::text FROM pg_catalog.pg_index WHERE indrelid=$1 AND indpred IS NOT NULL",
        &[&oid],
    ).await.map_err(catalog_error)?;
    if expressions.iter().any(|row| {
        let expression: String = row.get(0);
        let name: Option<String> = row.get(1);
        let typed_membership = name
            .as_ref()
            .and_then(|name| {
                table
                    .structure
                    .checks
                    .iter()
                    .find(|check| &check.name == name)
            })
            .and_then(|check| check.set_membership.as_ref());
        let safe_membership = typed_membership.is_some_and(|membership| {
            builtin_text_arrays.contains(&membership.column)
                && crate::verify::set_membership_matches(&expression, membership) == Some(true)
        });
        !safe_membership && !safe_expression(&expression, &names)
    }) {
        return Err(safety_error(
            "reject recovery requires a reviewed row-local CHECK/index expression",
        ));
    }
    Ok(())
}

fn validate(batch: &EncodedBatch) -> bool {
    let mut end = 0;
    let mut ordinal = 0;
    for row in &batch.rows {
        if row.start != end
            || row.end <= row.start
            || row.end > batch.storage.bytes.len()
            || batch.storage.bytes[row.end - 1] != b'\n'
            || row.locator.ordinal <= ordinal
        {
            return false;
        }
        end = row.end;
        ordinal = row.locator.ordinal;
    }
    end == batch.storage.bytes.len()
}
fn rollback_confirmed(target: &TargetConnection, failure: &TargetError) -> bool {
    failure.cause.is_none()
        && target.stage_handle().get() == CopyStage::Idle
        && matches!(
            failure.stage,
            CopyStage::CopyData | CopyStage::CopyFinish | CopyStage::CommitAttempt
        )
}
fn row_failure(failure: &TargetError) -> bool {
    failure.kind == FailureKind::RowData
        && matches!(failure.stage, CopyStage::CopyData | CopyStage::CopyFinish)
        && matches!(
            failure.sqlstate.as_deref(),
            Some(
                "22001"
                    | "22003"
                    | "22007"
                    | "22008"
                    | "22021"
                    | "22P02"
                    | "23502"
                    | "23505"
                    | "23514"
            )
        )
}

pub async fn copy_with_recovery(
    target: &mut TargetConnection,
    table: &TablePlan,
    batch: &EncodedBatch,
    context: &mut RecoveryContext<'_>,
) -> Result<RecoveryProgress, RecoveryError> {
    let mut core = CoreContext {
        policy: context.policy,
        artifacts: Some(context.artifacts),
        owner: None,
        table_index: 0,
        published: RecoveryProgress::default(),
        reject_budget: context.reject_budget,
        observer: context.observer,
    };
    recover(target, table, batch, &mut core).await
}

pub(super) struct DurableRecoveryContext<'a, 'r> {
    pub policy: RowErrorPolicy,
    pub owner: &'a super::ReportOwner<'r>,
    pub table_index: usize,
    pub reject_budget: &'a RejectBudget,
    pub observer: &'a mut (dyn FnMut(RecoveryProgress) -> io::Result<()> + Send),
}
struct CoreContext<'a, 'r> {
    policy: RowErrorPolicy,
    artifacts: Option<&'a RunArtifacts>,
    owner: Option<&'a super::ReportOwner<'r>>,
    table_index: usize,
    published: RecoveryProgress,
    reject_budget: &'a RejectBudget,
    observer: &'a mut (dyn FnMut(RecoveryProgress) -> io::Result<()> + Send),
}
pub(super) async fn copy_with_durable_recovery(
    target: &mut TargetConnection,
    table: &TablePlan,
    batch: &EncodedBatch,
    context: &mut DurableRecoveryContext<'_, '_>,
) -> Result<RecoveryProgress, RecoveryError> {
    let mut core = CoreContext {
        policy: context.policy,
        artifacts: None,
        owner: Some(context.owner),
        table_index: context.table_index,
        published: RecoveryProgress::default(),
        reject_budget: context.reject_budget,
        observer: context.observer,
    };
    recover(target, table, batch, &mut core).await
}
async fn observed(
    context: &mut CoreContext<'_, '_>,
    progress: RecoveryProgress,
) -> Result<(), RecoveryError> {
    (context.observer)(progress).map_err(|_| error(RecoveryFailure::ObserverIo, progress, None))?;
    if (progress.committed_rows != context.published.committed_rows
        || progress.rejected_rows != context.published.rejected_rows)
        && let Some(owner) = context.owner
    {
        owner
            .persist()
            .await
            .map_err(|_| error(RecoveryFailure::RejectIo, progress, None))?;
    }
    context.published = progress;
    Ok(())
}
async fn recover(
    target: &mut TargetConnection,
    table: &TablePlan,
    batch: &EncodedBatch,
    context: &mut CoreContext<'_, '_>,
) -> Result<RecoveryProgress, RecoveryError> {
    let total = batch.rows.len() as u64;
    let mut progress = RecoveryProgress {
        pending_rows: total,
        ..Default::default()
    };
    if !validate(batch) {
        return Err(error(RecoveryFailure::InvalidBatch, progress, None));
    }
    if total == 0 {
        return Ok(progress);
    }
    let mut safety_checked = false;
    if context.policy == RowErrorPolicy::Reject {
        check_recovery_safety(target, table)
            .await
            .map_err(|failure| error(RecoveryFailure::Database, progress, Some(failure)))?;
        safety_checked = true;
    }
    // ponytail: sequential depth-first ranges; retain one immutable byte allocation/permit.
    let mut pending: Vec<Range<usize>> = Vec::with_capacity(usize::BITS as usize);
    pending.push(0..batch.rows.len());
    while let Some(range) = pending.pop() {
        let count = range.len() as u64;
        let bytes = batch
            .storage
            .bytes
            .slice(batch.rows[range.start].start..batch.rows[range.end - 1].end);
        let mut attempt = 0;
        let failure = loop {
            progress.active_rows = count;
            progress.pending_rows =
                total - progress.committed_rows - progress.rejected_rows - count;
            observed(context, progress).await?;
            attempt += 1;
            match postgres::copy_bytes(target, table, bytes.clone(), count).await {
                Ok(acknowledged) => {
                    progress.committed_rows += acknowledged;
                    progress.active_rows = 0;
                    progress.pending_rows =
                        total - progress.committed_rows - progress.rejected_rows;
                    observed(context, progress).await?;
                    break None;
                }
                Err(failure) => {
                    if !rollback_confirmed(target, &failure) {
                        return Err(error(RecoveryFailure::Database, progress, Some(failure)));
                    }
                    progress.active_rows = 0;
                    progress.pending_rows =
                        total - progress.committed_rows - progress.rejected_rows;
                    observed(context, progress).await?;
                    if failure.kind == FailureKind::TransactionRetry
                        && matches!(failure.sqlstate.as_deref(), Some("40001" | "40P01"))
                    {
                        // Even stop mode cannot replay custom side effects based on a code alone.
                        if !safety_checked {
                            if let Err(mut safety) = check_recovery_safety(target, table).await {
                                safety.cause = Some(Box::new(failure));
                                return Err(error(
                                    RecoveryFailure::Database,
                                    progress,
                                    Some(safety),
                                ));
                            }
                            safety_checked = true;
                        }
                        if attempt == 3 {
                            return Err(error(
                                RecoveryFailure::RetryExhausted,
                                progress,
                                Some(failure),
                            ));
                        }
                        tokio::time::sleep(Duration::from_millis(20 * attempt)).await;
                    } else {
                        break Some(failure);
                    }
                }
            }
        };
        let Some(failure) = failure else {
            continue;
        };
        if context.policy != RowErrorPolicy::Reject || !row_failure(&failure) {
            return Err(error(RecoveryFailure::Database, progress, Some(failure)));
        }
        if range.len() > 1 {
            let middle = range.start + range.len() / 2;
            pending.push(middle..range.end);
            pending.push(range.start..middle);
            debug_assert!(pending.len() <= usize::BITS as usize);
        } else {
            let Some(ticket) = context.reject_budget.reserve() else {
                return Err(error(RecoveryFailure::RejectLimit, progress, Some(failure)));
            };
            let row = &batch.rows[range.start];
            let diagnostic = Diagnostic {
                code: "COPY_ROW_REJECTED".into(),
                stage: "copy".into(),
                object: Some(table.id.clone()),
                severity: Severity::Warning,
                message: format!(
                    "row rejected after confirmed rollback (SQLSTATE {})",
                    failure.sqlstate.as_deref().unwrap_or("unknown")
                ),
            };
            if let Some(owner) = context.owner {
                owner
                    .artifacts
                    .lease()
                    .await
                    .map_err(|_| error(RecoveryFailure::RejectIo, progress, None))?
                    .reject_copy(
                        context.table_index,
                        &row.locator,
                        batch.storage.clone(),
                        row.start..row.end,
                        &diagnostic,
                        ticket.0,
                    )
                    .await
                    .map_err(|_| error(RecoveryFailure::RejectIo, progress, None))?;
            } else {
                context
                    .artifacts
                    .expect("direct recovery artifacts")
                    .reject_copy(
                        &table.id,
                        &row.locator,
                        &batch.storage.bytes[row.start..row.end],
                        &diagnostic,
                    )
                    .map_err(|_| error(RecoveryFailure::RejectIo, progress, None))?;
                ticket.commit();
            }
            progress.rejected_rows += 1;
            progress.pending_rows = total - progress.committed_rows - progress.rejected_rows;
            observed(context, progress).await?;
        }
    }
    Ok(progress)
}

#[derive(Debug, PartialEq)]
enum Token {
    Name(String),
    QuotedName(String),
    Literal,
    Left,
    Right,
    Compare,
    And,
    Or,
    Not,
    Is,
    Null,
    Bool,
    Cast,
}
fn tokenize(input: &str) -> Option<Vec<Token>> {
    if input.len() > 65536 {
        return None;
    }
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        let token = match c {
            c if c.is_whitespace() => continue,
            '(' => Token::Left,
            ')' => Token::Right,
            ':' if chars.next()? == ':' => Token::Cast,
            '=' => Token::Compare,
            '<' | '>' => {
                if chars
                    .peek()
                    .is_some_and(|next| *next == '=' || (*next == '>' && c == '<'))
                {
                    chars.next();
                }
                Token::Compare
            }
            '!' if chars.next()? == '=' => Token::Compare,
            '\'' | '"' => {
                let mut value = String::new();
                loop {
                    let next = chars.next()?;
                    if next == c {
                        if chars.peek() == Some(&c) {
                            chars.next();
                            value.push(c);
                        } else {
                            break;
                        }
                    } else if next == '\\' && c == '\'' {
                        return None;
                    } else {
                        value.push(next);
                    }
                }
                if c == '"' {
                    Token::QuotedName(value)
                } else {
                    Token::Literal
                }
            }
            c if c.is_ascii_digit() || c == '-' || c == '+' => {
                if !c.is_ascii_digit() && !chars.peek().is_some_and(char::is_ascii_digit) {
                    return None;
                }
                while chars
                    .peek()
                    .is_some_and(|c| c.is_ascii_digit() || *c == '.')
                {
                    chars.next();
                }
                Token::Literal
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut name = c.to_string();
                while chars
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    name.push(chars.next()?);
                }
                match name.as_str() {
                    "AND" => Token::And,
                    "OR" => Token::Or,
                    "NOT" => Token::Not,
                    "IS" => Token::Is,
                    "NULL" => Token::Null,
                    "true" | "false" | "TRUE" | "FALSE" => Token::Bool,
                    _ => Token::Name(name),
                }
            }
            _ => return None,
        };
        tokens.push(token);
    }
    Some(tokens)
}
struct Expression<'a> {
    tokens: &'a [Token],
    position: usize,
    names: &'a BTreeSet<String>,
}
impl Expression<'_> {
    fn take(&mut self, token: &Token) -> bool {
        if self.tokens.get(self.position) == Some(token) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn parse(&mut self, minimum: u8, depth: usize) -> bool {
        if depth > 128 {
            return false;
        }
        if self.take(&Token::Not) {
            if !self.parse(4, depth + 1) {
                return false;
            }
        } else if self.take(&Token::Left) {
            if !self.parse(0, depth + 1) || !self.take(&Token::Right) {
                return false;
            }
        } else {
            match self.tokens.get(self.position) {
                Some(Token::Literal | Token::Null | Token::Bool) => self.position += 1,
                Some(Token::Name(name) | Token::QuotedName(name)) if self.names.contains(name) => {
                    self.position += 1
                }
                _ => return false,
            }
        }
        loop {
            let precedence = match self.tokens.get(self.position) {
                Some(Token::Or) => 1,
                Some(Token::And) => 2,
                Some(Token::Compare | Token::Is) => 3,
                Some(Token::Cast) => 5,
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            let token = &self.tokens[self.position];
            self.position += 1;
            if *token == Token::Is {
                self.take(&Token::Not);
                if !(self.take(&Token::Null) || self.take(&Token::Bool)) {
                    return false;
                }
            } else if *token == Token::Cast {
                let Some(Token::Name(name)) = self.tokens.get(self.position) else {
                    return false;
                };
                if !matches!(
                    name.as_str(),
                    "text"
                        | "varchar"
                        | "bpchar"
                        | "boolean"
                        | "bool"
                        | "smallint"
                        | "integer"
                        | "bigint"
                        | "int2"
                        | "int4"
                        | "int8"
                        | "numeric"
                        | "real"
                        | "float4"
                        | "float8"
                        | "date"
                        | "time"
                        | "timestamp"
                        | "timestamptz"
                        | "interval"
                        | "bytea"
                ) {
                    return false;
                }
                self.position += 1;
            } else if !self.parse(precedence + 1, depth + 1) {
                return false;
            }
        }
        true
    }
}
fn safe_expression(input: &str, names: &BTreeSet<String>) -> bool {
    let Some(tokens) = tokenize(input) else {
        return false;
    };
    let mut expression = Expression {
        tokens: &tokens,
        position: 0,
        names,
    };
    expression.parse(0, 0) && expression.position == tokens.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_expression_and_global_ticket_limits() {
        let names = BTreeSet::from(["n".into(), "odd name".into()]);
        for value in [
            "(n >= 0)",
            "((n < 4) OR (n IS NULL))",
            "NOT (\"odd name\" = 'a'::text)",
            "true",
            "-7",
        ] {
            assert!(safe_expression(value, &names), "{value}");
        }
        for value in [
            "spoof(n)",
            "random() > 0",
            "n + 1",
            "n OPERATOR(public.>=) 0",
            "n::public.domain",
            "n::\"integer\"",
            "(SELECT true)",
            "n;SELECT 1",
            "n >= 0 garbage",
        ] {
            assert!(!safe_expression(value, &names), "{value}");
        }
        let budget = RejectBudget::new(1);
        let ticket = budget.reserve().unwrap();
        assert!(budget.reserve().is_none());
        drop(ticket);
        budget.reserve().unwrap().commit();
        assert_eq!(budget.used(), 1);
        assert!(budget.reserve().is_none());
    }
}
