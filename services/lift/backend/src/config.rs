use std::env;

pub struct Config {
    pub bind: String,
    pub database_url: String,
    pub static_dir: String,
}

impl Config {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();

        Self {
            bind: env::var("LIFT_BIND").unwrap_or_else(|_| "127.0.0.1:3033".to_string()),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:lift.db?mode=rwc".to_string()),
            static_dir: env::var("LIFT_STATIC_DIR")
                .unwrap_or_else(|_| "services/lift/frontend/dist".to_string()),
        }
    }
}
