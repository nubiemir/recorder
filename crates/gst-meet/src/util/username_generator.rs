use nanoid::nanoid;

pub fn generate_username() -> String {
    let stats_id = nanoid!();
    stats_id
}
