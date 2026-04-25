//! Async MySQL wire server — Phase 1 scaffold (feature = "async-proxy").
//!
//! Handles only:
//!   • COM_PING  → OK
//!   • SELECT 1 / SELECT @@version_comment / SELECT VERSION()  → single-row echo
//!
//! Phase 2/3/4 will port the full query path from server.rs.

use async_trait::async_trait;
use opensrv_mysql::{
    AsyncMysqlShim, Column, ColumnFlags, ColumnType, ErrorKind, OkResponse, ParamParser,
    QueryResultWriter, StatementMetaWriter,
};
use std::io;
use tokio::io::AsyncWrite;

const SERVER_VERSION: &str = "8.0.37-synapse-async";

pub struct AsyncHandler;

#[async_trait]
impl<W: AsyncWrite + Send + Unpin> AsyncMysqlShim<W> for AsyncHandler {
    type Error = io::Error;

    async fn on_prepare<'a>(
        &'a mut self,
        _query: &'a str,
        info: StatementMetaWriter<'a, W>,
    ) -> io::Result<()> {
        info.reply(0, &[], &[]).await
    }

    async fn on_execute<'a>(
        &'a mut self,
        _id: u32,
        _params: ParamParser<'a>,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        results.completed(OkResponse::default()).await
    }

    async fn on_close(&mut self, _stmt: u32) {}

    async fn on_query<'a>(
        &'a mut self,
        sql: &'a str,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        let sql_upper = sql.trim().to_uppercase();

        let value: Option<&str> = if sql_upper.is_empty()
            || sql_upper == "PING"
            || sql_upper.starts_with("SELECT 1")
        {
            Some("1")
        } else if sql_upper.contains("@@VERSION_COMMENT") {
            Some("synapse-async")
        } else if sql_upper.contains("VERSION()") || sql_upper.contains("@@VERSION") {
            Some(SERVER_VERSION)
        } else {
            None
        };

        if let Some(val) = value {
            let cols = [Column {
                table: String::new(),
                column: "value".to_string(),
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
            }];
            let mut rw = results.start(&cols).await?;
            rw.write_row(std::iter::once(val)).await?;
            rw.finish().await
        } else {
            results
                .error(
                    ErrorKind::ER_NOT_SUPPORTED_YET,
                    b"async-proxy: read-only smoke mode",
                )
                .await
        }
    }
}
