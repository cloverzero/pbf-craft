use clap::Args;
use pbf_craft::writers::PbfWriter;

use crate::db::DatabaseReader;

#[derive(Args)]
pub struct ExportCommand {
    /// output path
    #[clap(short, long, value_parser)]
    output: String,

    /// database user
    #[clap(long, value_parser)]
    user: String,

    /// password (falls back to the PGPASSWORD environment variable)
    #[clap(long, value_parser)]
    password: Option<String>,

    /// the host of the database
    #[clap(long, value_parser)]
    host: String,

    /// the port of the database
    #[clap(long, value_parser, default_value_t = 5432)]
    port: u16,

    /// the database name
    #[clap(long, value_parser)]
    dbname: String,
}

impl ExportCommand {
    pub fn run(self) -> anyhow::Result<()> {
        // Note: the password is deliberately not printed.
        blue!("Exporting ");
        dark_yellow!(
            "postgres://{}@{}:{}/{}",
            &self.user,
            &self.host,
            &self.port,
            &self.dbname
        );
        blue!(" to ");
        dark_yellow!("{}", self.output);
        println!(" ...");

        let password = self
            .password
            .or_else(|| std::env::var("PGPASSWORD").ok())
            .ok_or_else(|| {
                anyhow!("no password provided: pass --password or set the PGPASSWORD environment variable")
            })?;
        let db_reader = DatabaseReader::new(self.host, self.port, self.dbname, self.user, password);
        let mut writer = PbfWriter::from_path(&self.output, true)?;
        db_reader.read(|el_container| writer.write(el_container))?;
        writer.finish()?;
        Ok(())
    }
}
