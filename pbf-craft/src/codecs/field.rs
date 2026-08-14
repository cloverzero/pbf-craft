use crate::proto::osmformat::PrimitiveBlock;
use chrono::{DateTime, Utc};

/// Decodes/encodes the per-block field scalars (coordinates, timestamps, strings).
///
/// Decoding is fallible: a malformed block can carry a zero/negative granularity (division by
/// zero) or values whose scaled product overflows `i64`. All decode methods therefore return
/// `anyhow::Result`, and `new_with_block` rejects invalid blocks up front.
pub struct FieldCodec {
    date_granularity: i32,
    granularity: i32,
    lat_offset: i64,
    lon_offset: i64,
    string_table: Vec<String>,
}

impl FieldCodec {
    /// Creates a codec for the writer side. The writer always uses the PBF defaults
    /// (granularity 100, date_granularity 1000, zero offsets), so the values are never zero.
    pub fn new(granularity: i32, date_granularity: i32) -> Self {
        Self {
            date_granularity,
            granularity,
            lat_offset: 0,
            lon_offset: 0,
            string_table: Vec::new(),
        }
    }

    /// Creates a codec from a decoded `PrimitiveBlock`, validating the granularity values.
    pub fn new_with_block(block: &PrimitiveBlock) -> anyhow::Result<Self> {
        let granularity = block.get_granularity();
        let date_granularity = block.get_date_granularity();
        if granularity <= 0 {
            bail!("invalid block granularity: {}", granularity);
        }
        if date_granularity <= 0 {
            bail!("invalid block date_granularity: {}", date_granularity);
        }
        let bytes_array = block.get_stringtable().get_s();
        let string_table = if bytes_array.is_empty() {
            Vec::with_capacity(0)
        } else {
            bytes_array
                .iter()
                .map(|bytes| match String::from_utf8(bytes.clone()) {
                    Ok(str) => str,
                    Err(err) => {
                        eprintln!("{}", err);
                        String::new()
                    }
                })
                .collect::<Vec<String>>()
        };
        Ok(Self {
            date_granularity,
            granularity,
            lat_offset: block.get_lat_offset(),
            lon_offset: block.get_lon_offset(),
            string_table,
        })
    }

    pub fn encode_latitude(&self, latitude: i64) -> i64 {
        (latitude - self.lat_offset) / self.granularity as i64
    }

    pub fn decode_latitude(&self, raw_latitude: i64) -> anyhow::Result<i64> {
        let scaled = (self.granularity as i64)
            .checked_mul(raw_latitude)
            .ok_or_else(|| anyhow!("latitude value overflows i64: {}", raw_latitude))?;
        self.lat_offset
            .checked_add(scaled)
            .ok_or_else(|| anyhow!("latitude offset overflows i64"))
    }

    pub fn encode_longitude(&self, longitude: i64) -> i64 {
        (longitude - self.lon_offset) / self.granularity as i64
    }

    pub fn decode_longitude(&self, raw_longitude: i64) -> anyhow::Result<i64> {
        let scaled = (self.granularity as i64)
            .checked_mul(raw_longitude)
            .ok_or_else(|| anyhow!("longitude value overflows i64: {}", raw_longitude))?;
        self.lon_offset
            .checked_add(scaled)
            .ok_or_else(|| anyhow!("longitude offset overflows i64"))
    }

    pub fn encode_timestamp(&self, time: DateTime<Utc>) -> i64 {
        time.timestamp_millis() / self.date_granularity as i64
    }

    pub fn decode_timestamp(&self, raw_timestamp: i64) -> anyhow::Result<DateTime<Utc>> {
        let timestamp = (self.date_granularity as i64)
            .checked_mul(raw_timestamp)
            .ok_or_else(|| anyhow!("timestamp value overflows i64: {}", raw_timestamp))?;
        DateTime::from_timestamp_millis(timestamp)
            .ok_or_else(|| anyhow!("timestamp out of range: {}", timestamp))
    }

    pub fn decode_string(&self, string_id: usize) -> String {
        match self.string_table.get(string_id) {
            None => {
                eprintln!("no matched string table id: {}", string_id);
                String::new()
            }
            Some(s) => s.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::osmformat::PrimitiveBlock;

    fn block_with(granularity: i32, date_granularity: i32) -> PrimitiveBlock {
        let mut block = PrimitiveBlock::new();
        block.set_granularity(granularity);
        block.set_date_granularity(date_granularity);
        block
    }

    #[test]
    fn new_with_block_rejects_zero_or_negative_granularity() {
        assert!(FieldCodec::new_with_block(&block_with(0, 1000)).is_err());
        assert!(FieldCodec::new_with_block(&block_with(-100, 1000)).is_err());
        assert!(FieldCodec::new_with_block(&block_with(100, 0)).is_err());
        assert!(FieldCodec::new_with_block(&block_with(100, -1000)).is_err());
    }

    #[test]
    fn new_with_block_accepts_valid_granularity() {
        assert!(FieldCodec::new_with_block(&block_with(100, 1000)).is_ok());
    }

    #[test]
    fn decode_latitude_overflow_is_an_error_not_a_panic() {
        let codec = FieldCodec::new_with_block(&block_with(100, 1000)).unwrap();
        // granularity * raw overflows i64 for extreme values.
        assert!(codec.decode_latitude(i64::MAX).is_err());
        assert!(codec.decode_latitude(i64::MIN).is_err());
        // Offset overflow: lat_offset + scaled overflows.
        let mut block = block_with(100, 1000);
        block.set_lat_offset(i64::MAX);
        let codec = FieldCodec::new_with_block(&block).unwrap();
        assert!(codec.decode_latitude(1).is_err());
    }

    #[test]
    fn decode_timestamp_overflow_is_an_error_not_a_panic() {
        let codec = FieldCodec::new_with_block(&block_with(100, 1000)).unwrap();
        assert!(codec.decode_timestamp(i64::MAX).is_err());
        assert!(codec.decode_timestamp(i64::MIN).is_err());
    }

    #[test]
    fn roundtrip_scalars() {
        let codec = FieldCodec::new_with_block(&block_with(100, 1000)).unwrap();
        // Values divisible by the granularity roundtrip exactly.
        let lat = 1_234_567_800;
        let lon = -987_654_300;
        assert_eq!(
            codec.decode_latitude(codec.encode_latitude(lat)).unwrap(),
            lat
        );
        assert_eq!(
            codec.decode_longitude(codec.encode_longitude(lon)).unwrap(),
            lon
        );
    }
}
