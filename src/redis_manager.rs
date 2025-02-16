use std::fs::File;

use csv::{Reader, StringRecord};
use redis::{Client, Commands, Connection, RedisResult};
use serde::de::DeserializeOwned;
use serde::export::fmt::Display;
use serde::Serialize;

use crate::geocoding::{COORDINATES_SEPARATOR, POSTCODE_TABLE_NAME};

fn connect_and_query<F, T>(mut action: F) -> Option<T>
where
    F: FnMut(Connection) -> Option<T>,
{
    let client: Client = get_redis_client().ok()?;
    let con = client.get_connection().ok()?;
    action(con)
}

// TODO [#30]: add concurrency to all of this once benchmarked
fn get_redis_client() -> RedisResult<Client> {
    redis::Client::open("redis://127.0.0.1/")
}

pub fn get_coordinates(postcode: &str) -> Option<String> {
    connect_and_query(|mut connection| connection.hget(POSTCODE_TABLE_NAME, postcode).ok()?)
}

pub fn get_postcode(coordinates: Vec<f64>) -> Option<String> {
    let coord_string = coordinates
        .iter()
        .map(|coord| coord.to_string())
        .collect::<Vec<String>>()
        .join(COORDINATES_SEPARATOR);

    // TODO [#31]: fix this
    connect_and_query(|mut connection| {
        redis::cmd("HSCAN")
            .arg(&["0", "MATCH", &coord_string])
            .query(&mut connection)
            .ok()?
    })
}

pub fn get<T: DeserializeOwned>(table: &str, key: &str) -> Option<T> {
    let result: Option<String> =
        connect_and_query(|mut connection| connection.hget(table, key).ok()?);

    match result {
        None => None,
        Some(res) => serde_json::from_str(res.as_str()).ok()?,
    }
}

pub fn del(table: &str, key: &str) -> Option<String> {
    connect_and_query(|mut connection| connection.hdel(table, key).ok()?)
}

pub fn set<T: Serialize + Display>(table: &str, key: &str, value: T) -> Option<String> {
    let client: Client = get_redis_client().expect("Unable to get a redis client");
    let mut con = client.get_connection().expect("Unable to get a connection");

    let result: RedisResult<i32> = con.hset(
        table,
        key,
        serde_json::to_string(&value).expect("Unable to serialize value"),
    );

    match result {
        Err(err) => {
            log::error!("Couldn't write to redis, reason: {:?}", err.detail());
            None
        }
        Ok(res) => {
            let msg = format!(
                "Wrote {} to table: {} with key {} and result {}",
                value, table, key, res
            );
            log::debug!("{}", msg);
            Some(msg)
        }
    }
}

pub fn count(table: &str) -> i32 {
    let client: Client = get_redis_client().unwrap();
    let mut con = client.get_connection().unwrap();

    con.hlen(table).unwrap()
}

// TODO [#46]: decouple this
pub fn bulk_set(csv: &mut Reader<File>, key: &str) -> Option<()> {
    let records = csv.records();
    let client: Client = get_redis_client().unwrap();
    let mut con = client.get_connection().unwrap();

    let postcode_index = 0;
    let lat_index = 1;
    let lon_index = 2;

    let mut count = 0;
    let mut pipeline = redis::pipe();

    // TODO [#32]: use rayon to parallelise this
    records.for_each(|row| {
        let row = &row.unwrap();
        count += 1;
        pipeline
            .hset(
                key,
                build_row_field(postcode_index, row),
                build_row_value(lat_index, lon_index, row),
            )
            .ignore();
    });

    let result: RedisResult<()> = pipeline.query(&mut con);

    match result {
        Ok(res) => {
            log::info!(
                "Finished bootstrapping {} postcodes, result: {:?}",
                count,
                res
            );
            Some(())
        }
        Err(err) => {
            log::error!("Failed to write to postcodes, error: {}", err);
            None
        }
    }
}

// TODO [#47]: move these away
fn build_row_value(lat_index: usize, lon_index: usize, row: &StringRecord) -> String {
    format!(
        "{};{}",
        row.get(lat_index).unwrap(),
        row.get(lon_index).unwrap()
    )
}

fn build_row_field(postcode_index: usize, row: &StringRecord) -> String {
    row.get(postcode_index)
        .unwrap()
        .to_string()
        .replace(" ", "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_bulk_set() {
        let file_name = "./test.bulk.set.csv";

        let test_file = File::create(&file_name).expect("Unable to create ./test.csv");
        test_file.set_len(0).unwrap();
        let mut writer = csv::Writer::from_path(&file_name).expect("Issue reading test.csv");
        writer
            .write_record(&["TEST1", "0.0", "0.0"])
            .expect("Unable to write test record");
        let mut reader = csv::Reader::from_path(&file_name).expect("Issue reading test.csv");
        let set_count = bulk_set(&mut reader, POSTCODE_TABLE_NAME);
        fs::remove_file(&file_name).unwrap();
        assert_eq!(set_count, Some(()));
    }

    #[test]
    fn test_count() {
        set("TEST_TABLE_COUNT", "TEST", "TEST").unwrap();
        let table_count = count("TEST_TABLE_COUNT");
        assert_ne!(table_count, 0);
    }

    #[test]
    fn test_count_0() {
        let table_count = count("TEST");
        assert_eq!(table_count, 0);
    }

    #[test]
    fn test_set() {
        del("TEST_TABLE", "TEST");
        let result = set("TEST_TABLE", "TEST", "TEST").unwrap();
        assert_eq!(
            result,
            "Wrote TEST to table: TEST_TABLE with key TEST and result 1"
        );
    }

    #[test]
    fn test_del() {
        let table_count = count("TEST_DEL_TABLE");
        println!("{}", table_count);
        if table_count == 0 {
            del("TEST_DEL_TABLE", "TEST");
        }
        set("TEST_DEL_TABLE", "TEST", "TEST").unwrap();
        del("TEST_DEL_TABLE", "TEST");
        let table_count = count("TEST_DEL_TABLE");
        assert_eq!(table_count, 0);
    }

    #[test]
    fn test_get() {
        set("TEST_GET_TABLE", "TEST", "TEST").unwrap();
        let get: String = get("TEST_GET_TABLE", "TEST").unwrap();
        assert_eq!(get, "TEST")
    }

    #[test]
    fn test_get_postcode() {}

    #[test]
    fn test_get_coordinates() {
        let key = "IMAGINARYPOSTCODE";
        del(POSTCODE_TABLE_NAME, key);
        set(POSTCODE_TABLE_NAME, key, "0.0;0.0").unwrap();
        let coordinates = get_coordinates(key).unwrap();
        assert_eq!(coordinates, "\"0.0;0.0\"")
    }

    #[test]
    fn test_get_redis_client() {
        assert!(get_redis_client().is_ok())
    }

    #[test]
    fn test_connect_and_query() {
        let result: Option<String> =
            connect_and_query(|mut connection| connection.set("TEST_HOF", "TEST_HOF").ok()?);
        assert!(result.is_some());
    }
}
