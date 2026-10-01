//! GO/NO-GO: libSQL + FTS5 + sqlite-vec coexist in same process?
//! Run: cargo run --example libsql_fts5_abi_check -p synapse-core --features backend-libsql
//!
//! RESULT (libsql 0.9.30, 2026-05-05): FAIL
//! Root cause: libsql-rusqlite (internal dep of libsql-sys) calls
//!   sqlite3_config(SQLITE_CONFIG_MULTITHREAD) + sqlite3_initialize()
//!   before libsql's own Database::new() can call
//!   sqlite3_config(SQLITE_CONFIG_SERIALIZED) — which then returns
//!   SQLITE_MISUSE (21) and panics.
//! Both share the same libsql-ffi bundled SQLite, so threading config
//! cannot be set twice. This is an upstream conflict in libsql 0.9.x.
//!
//! Phase 2 verdict: PAUSE — use libsql 0.6.x or wait for upstream fix.

#[cfg(not(feature = "backend-libsql"))]
fn main() {
    eprintln!("SKIP: compile with --features backend-libsql");
    std::process::exit(2);
}

#[cfg(feature = "backend-libsql")]
fn main() {
    // Attempt to set SQLITE_CONFIG_SERIALIZED before any init.
    // In libsql 0.9.x this fails because libsql-rusqlite (internal dep)
    // already called sqlite3_initialize() via SQLITE_CONFIG_MULTITHREAD.
    unsafe {
        let rc = libsql::ffi::sqlite3_config(libsql::ffi::SQLITE_CONFIG_SERIALIZED);
        if rc != 0 {
            // SQLITE_MISUSE = 21: already initialized
            eprintln!(
                "FAIL: sqlite3_config(SQLITE_CONFIG_SERIALIZED) returned {}",
                rc
            );
            eprintln!("Root cause: libsql-rusqlite (libsql internal dep) pre-initializes");
            eprintln!("  shared libsql-ffi SQLite with MULTITHREAD before this call.");
            eprintln!("  libsql 0.9.x threading conflict — upstream bug.");
            eprintln!("VERDICT: Phase 2 PAUSE");
            eprintln!("  Use libsql <= 0.6.x (no libsql-rusqlite dep) or await upstream fix.");
            std::process::exit(1);
        }

        // If config succeeded, register sqlite-vec extension.
        libsql::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut libsql::ffi::sqlite3,
                *mut *const i8,
                *const libsql::ffi::sqlite3_api_routines,
            ) -> i32,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main())
        .unwrap();
}

#[cfg(feature = "backend-libsql")]
async fn async_main() -> anyhow::Result<()> {
    let db = libsql::Builder::new_local(":memory:").build().await?;
    let conn = db.connect()?;

    // FTS5 check
    conn.execute("CREATE VIRTUAL TABLE t USING fts5(body)", ())
        .await?;
    conn.execute("INSERT INTO t VALUES ('hello world')", ())
        .await?;
    let mut rows = conn
        .query("SELECT body FROM t WHERE body MATCH 'hello'", ())
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row returned"))?;
    let body: String = row.get(0)?;
    assert_eq!(body, "hello world", "FTS5 returned wrong row");

    // sqlite-vec check
    conn.execute(
        "CREATE VIRTUAL TABLE v USING vec0(id INTEGER PRIMARY KEY, e FLOAT[4])",
        (),
    )
    .await?;

    println!("OK: FTS5 + sqlite-vec both operational on libsql in-memory");
    println!("VERDICT: Phase 2 GO");
    Ok(())
}
