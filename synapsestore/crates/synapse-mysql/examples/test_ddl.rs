fn main() {
    let ddl = "CREATE TABLE wp_options (
option_id bigint(20) unsigned NOT NULL auto_increment,
option_name varchar(191) NOT NULL default '',
option_value longtext NOT NULL,
autoload varchar(20) NOT NULL default 'yes',
PRIMARY KEY  (option_id),
UNIQUE KEY option_name (option_name),
KEY autoload (autoload)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
    let out = synapse_mysql::rewrite::rewrite_ddl(ddl).unwrap();
    for s in &out {
        println!("--- stmt ---\n{}", s);
    }
}
