fn main() -> std::io::Result<()> {
    foundry_build::DatabaseCodegen::new()
        .migration_dir("database/migrations")
        .seeder_dir("database/seeders")
        .generate()
}
