use crate::acl::Acl;
use crate::rewrite::rewrite;
use anyhow::Result;
use msql_srv::*;
use std::collections::HashMap;
use std::io;

pub struct SynapseMySql {
    pub store: synapse_core::Store,
    pub acl: Acl,
    pub mode: String,
    pub current_db: Option<String>,
    pub stmts: HashMap<u32, String>,
    pub stmt_id_seq: u32,
}

impl SynapseMySql {
    pub fn new(store: synapse_core::Store, acl: Acl, mode: &str) -> Result<Self> {
        Ok(Self {
            store,
            acl,
            mode: mode.to_string(),
            current_db: None,
            stmts: HashMap::new(),
            stmt_id_seq: 0,
        })
    }
}

fn map_type(sqlite_typ: &str) -> ColumnType {
    let upper = sqlite_typ.to_uppercase();
    if upper.contains("INT") {
        ColumnType::MYSQL_TYPE_LONGLONG
    } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        ColumnType::MYSQL_TYPE_DOUBLE
    } else if upper.contains("BLOB") {
        ColumnType::MYSQL_TYPE_BLOB
    } else {
        ColumnType::MYSQL_TYPE_VAR_STRING
    }
}

fn rusqlite_to_string(v: &rusqlite::types::Value) -> String {
    match v {
        rusqlite::types::Value::Null => String::new(),
        rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        rusqlite::types::Value::Text(s) => s.clone(),
        rusqlite::types::Value::Blob(b) => String::from_utf8_lossy(b).to_string(),
    }
}

impl<W: io::Read + io::Write> MysqlShim<W> for SynapseMySql {
    type Error = io::Error;

    fn on_init(&mut self, database: &str, writer: InitWriter<W>) -> io::Result<()> {
        self.current_db = Some(database.to_string());
        writer.ok()
    }

    fn on_query(&mut self, query: &str, writer: QueryResultWriter<W>) -> io::Result<()> {
        let upper = query.trim().to_uppercase();

        if !self.acl.check_grant("root", &upper).unwrap_or(true) {
            return writer.error(ErrorKind::ER_ACCESS_DENIED_ERROR, b"access denied");
        }

        if upper.starts_with("CALL ") {
            return handle_call(self, query, writer);
        }

        let sql = match rewrite(query, &self.mode) {
            Ok(s) => s,
            Err(e) => return writer.error(ErrorKind::ER_UNKNOWN_ERROR, format!("rewrite: {}", e).as_bytes()),
        };

        let conn = &mut self.store.conn;
        let is_select = sql.trim().to_uppercase().starts_with("SELECT")
            || sql.trim().to_uppercase().starts_with("PRAGMA")
            || sql.trim().to_uppercase().starts_with("SHOW");

        if is_select {
            let sync_result: Result<(Vec<Column>, Vec<Vec<String>>), String> = (|| {
                let mut stmt = conn.prepare(&sql).map_err(|e| format!("{}", e))?;
                let cols = stmt.columns();
                let cols_count = cols.len();
                let mut col_defs: Vec<Column> = Vec::with_capacity(cols_count);
                for col in &cols {
                    let name = col.name().to_string();
                    let typ = col.decl_type().unwrap_or("TEXT");
                    col_defs.push(Column {
                        table: String::new(),
                        column: name,
                        coltype: map_type(typ),
                        colflags: ColumnFlags::empty(),
                    });
                }
                let mut rows: Vec<Vec<String>> = Vec::new();
                let mut r = stmt.query([]).map_err(|e| format!("{}", e))?;
                while let Ok(Some(row)) = r.next() {
                    let mut vals = Vec::with_capacity(cols_count);
                    for i in 0..cols_count {
                        let v: rusqlite::types::Value = row.get(i).unwrap_or(rusqlite::types::Value::Null);
                        vals.push(rusqlite_to_string(&v));
                    }
                    rows.push(vals);
                }
                Ok((col_defs, rows))
            })();

            match sync_result {
                Ok((col_defs, rows)) => {
                    let mut rw = writer.start(&col_defs)?;
                    for row in rows {
                        rw.write_row(row.iter().map(|s| s.as_str()))?;
                    }
                    rw.finish()
                }
                Err(msg) => {
                    writer.error(ErrorKind::ER_UNKNOWN_ERROR, msg.as_bytes())
                }
            }
        } else {
            let (affected, last_id) = match conn.execute(&sql, []) {
                Ok(n) => (n as u64, conn.last_insert_rowid() as u64),
                Err(e) => {
                    return writer.error(ErrorKind::ER_UNKNOWN_ERROR, format!("{}", e).as_bytes());
                }
            };
            writer.completed(affected, last_id)
        }
    }

    fn on_prepare(&mut self, query: &str, writer: StatementMetaWriter<W>) -> io::Result<()> {
        let id = self.stmt_id_seq;
        self.stmt_id_seq += 1;
        self.stmts.insert(id, query.to_string());
        let dummy: Vec<Column> = vec![];
        writer.reply(id, &dummy, &dummy)
    }

    fn on_execute(
        &mut self,
        id: u32,
        _params: msql_srv::ParamParser,
        writer: QueryResultWriter<W>,
    ) -> io::Result<()> {
        let query = match self.stmts.get(&id) {
            Some(q) => q.clone(),
            None => {
                return writer.error(ErrorKind::ER_UNKNOWN_STMT_HANDLER, b"unknown stmt");
            }
        };
        self.on_query(&query, writer)
    }

    fn on_close(&mut self, id: u32) {
        self.stmts.remove(&id);
    }
}

fn handle_call<W: io::Read + io::Write>(
    shim: &mut SynapseMySql,
    query: &str,
    writer: QueryResultWriter<W>,
) -> io::Result<()> {
    let body = query.trim()[5..].trim();
    let name = body.split('(').next().unwrap_or(body).trim();
    let sql = format!("SELECT body FROM _mysql_proc WHERE name = '{}'", name.replace("'", "''"));
    let proc_body: Result<String, _> = shim.store.conn.query_row(&sql, [], |row| row.get(0));
    match proc_body {
        Ok(body) => {
            shim.on_query(&body, writer)
        }
        Err(_) => writer.error(ErrorKind::ER_SP_DOES_NOT_EXIST, b"procedure not found"),
    }
}
