use std::fs::File;

use parquet_editor::document::Document;
use polars::prelude::{Column, DataFrame, DataType, NamedFrom, ParquetWriter, Series};
use pretty_assertions::assert_eq;

fn dataset() -> Document {
    let mut doc = Document::new(None);
    doc.add_column("text", DataType::String).unwrap();
    doc.add_column("score", DataType::Int64).unwrap();
    doc.add_column("accepted", DataType::Boolean).unwrap();
    doc.add_row().unwrap();
    doc.add_row().unwrap();
    doc
}

#[test]
fn parquet_round_trip_retains_unicode_newlines_empty_strings_and_nulls() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("日本語.parquet");
    let mut doc = dataset();
    doc.set_cell(0, 0, Some("花壇の物語🌸\n次の行\t\"quote\"\r\n"))
        .unwrap();
    doc.set_cell(1, 0, Some("")).unwrap();
    doc.set_cell(0, 1, Some("9223372036854775807")).unwrap();
    doc.set_cell(0, 2, Some("true")).unwrap();
    doc.save(&path, false).unwrap();
    assert!(!doc.is_dirty());
    let loaded = Document::open(&path).unwrap();
    assert_eq!((loaded.rows(), loaded.columns()), (2, 3));
    assert_eq!(
        loaded.cell(0, 0).unwrap(),
        Some("花壇の物語🌸\n次の行\t\"quote\"\r\n".into())
    );
    assert_eq!(loaded.cell(1, 0).unwrap(), Some("".into()));
    assert_eq!(loaded.cell(1, 1).unwrap(), None);
    assert_eq!(
        loaded.cell(0, 1).unwrap(),
        Some("9223372036854775807".into())
    );
    assert_eq!(
        loaded.frame().column("score").unwrap().dtype(),
        &DataType::Int64
    );
    assert_eq!(
        loaded.frame().column("accepted").unwrap().dtype(),
        &DataType::Boolean
    );
}

#[test]
fn invalid_or_overflowing_input_does_not_change_data_or_undo_history() {
    let mut doc = dataset();
    let history = doc.undo_len();
    for input in ["hello", "1.5", "9223372036854775808"] {
        assert!(doc.set_cell(0, 1, Some(input)).is_err());
        assert_eq!(doc.undo_len(), history);
        assert_eq!(doc.cell(0, 1).unwrap(), None);
    }
    assert!(doc.set_cell(0, 2, Some("perhaps")).is_err());
    doc.set_cell(0, 2, Some("FALSE")).unwrap();
    assert_eq!(doc.cell(0, 2).unwrap(), Some("false".into()));
}

#[test]
fn floating_point_edit_text_preserves_all_significant_digits() {
    let mut doc = Document::new(None);
    doc.add_column("number", DataType::Float64).unwrap();
    doc.add_row().unwrap();
    for value in [
        "1.2345678901234567",
        "0.000000000000000123456789",
        "1.7976931348623157e308",
    ] {
        doc.set_cell(0, 0, Some(value)).unwrap();
        let text = doc.cell(0, 0).unwrap().unwrap();
        assert_eq!(text.parse::<f64>().unwrap(), value.parse::<f64>().unwrap());
        assert!(!doc.set_cell(0, 0, Some(&text)).unwrap());
    }
}

#[test]
fn undo_tracks_the_saved_revision_and_new_edits_after_undo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data.parquet");
    let mut doc = dataset();
    doc.save(&path, false).unwrap();
    doc.set_cell(0, 0, Some("first")).unwrap();
    assert!(doc.is_dirty());
    assert!(doc.undo());
    assert!(!doc.is_dirty());
    doc.set_cell(0, 0, Some("second")).unwrap();
    doc.save(&path, true).unwrap();
    assert!(doc.undo());
    assert!(doc.is_dirty());
    doc.set_cell(0, 0, Some("third")).unwrap();
    assert!(doc.is_dirty());
    assert_eq!(
        Document::open(&path).unwrap().cell(0, 0).unwrap(),
        Some("second".into())
    );
}

#[test]
fn overwrite_and_save_failure_preserve_the_existing_file_and_dirty_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data.parquet");
    let mut doc = dataset();
    doc.save(&path, false).unwrap();
    let before = std::fs::read(&path).unwrap();
    doc.set_cell(0, 0, Some("new content")).unwrap();
    assert!(doc.save(&path, false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(doc.is_dirty());
    assert!(
        doc.save(&directory.path().join("missing/data.parquet"), false)
            .is_err()
    );
    assert_eq!(doc.path.as_ref(), Some(&path));
    assert!(doc.is_dirty());
    doc.save(&path, true).unwrap();
    assert!(!doc.is_dirty());
    assert_eq!(
        Document::open(&path).unwrap().cell(0, 0).unwrap(),
        Some("new content".into())
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn complex_and_temporal_columns_survive_edits_and_appended_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested.parquet");
    let dates = Column::new("date".into(), [Some(20000_i32), None])
        .strict_cast(&DataType::Date)
        .unwrap();
    let nested = Series::new(
        "tokens".into(),
        [
            Series::new("".into(), [1_i64, 2]),
            Series::new("".into(), [3_i64]),
        ],
    );
    let mut frame = DataFrame::new(
        2,
        vec![dates, nested.into(), Column::new("text".into(), ["a", "b"])],
    )
    .unwrap();
    ParquetWriter::new(File::create(&path).unwrap())
        .finish(&mut frame)
        .unwrap();
    let mut doc = Document::open(&path).unwrap();
    let dates = doc.frame().column("date").unwrap().clone();
    let nested = doc.frame().column("tokens").unwrap().clone();
    assert!(doc.set_cell(0, 0, Some("2026-10-17")).is_err());
    assert!(doc.set_cell(0, 1, Some("[99]")).is_err());
    doc.set_cell(0, 2, Some("changed")).unwrap();
    doc.add_row().unwrap();
    doc.save(&path, true).unwrap();
    let loaded = Document::open(&path).unwrap();
    assert!(
        loaded
            .frame()
            .column("date")
            .unwrap()
            .slice(0, 2)
            .equals_missing(&dates)
    );
    assert!(
        loaded
            .frame()
            .column("tokens")
            .unwrap()
            .slice(0, 2)
            .equals_missing(&nested)
    );
    assert_eq!(loaded.cell(2, 0).unwrap(), None);
    assert_eq!(loaded.cell(2, 1).unwrap(), None);
}

#[test]
fn new_columns_are_nullable_and_names_are_validated() {
    let mut doc = dataset();
    let history = doc.undo_len();
    assert!(doc.add_column("text", DataType::String).is_err());
    assert!(doc.add_column("  ", DataType::String).is_err());
    assert_eq!(doc.undo_len(), history);
    doc.add_column("新しい列", DataType::UInt64).unwrap();
    assert_eq!(doc.cell(0, 3).unwrap(), None);
    assert!(doc.set_cell(0, 3, Some("-1")).is_err());
    doc.set_cell(0, 3, Some("18446744073709551615")).unwrap();
    assert_eq!(doc.cell(0, 3).unwrap(), Some("18446744073709551615".into()));
}

#[test]
fn empty_typed_dataset_can_be_saved_and_undo_is_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty.parquet");
    let mut doc = Document::new(None);
    assert!(doc.add_row().is_err());
    assert!(doc.save(&path, false).is_err());
    doc.add_column("text", DataType::String).unwrap();
    doc.save(&path, false).unwrap();
    let loaded = Document::open(&path).unwrap();
    assert_eq!((loaded.rows(), loaded.columns()), (0, 1));
    doc.add_row().unwrap();
    for i in 0..150 {
        doc.set_cell(0, 0, Some(&i.to_string())).unwrap();
    }
    assert_eq!(doc.undo_len(), 100);
    for _ in 0..100 {
        assert!(doc.undo());
    }
    assert!(!doc.undo());
}

#[test]
fn narrow_integer_columns_keep_their_original_type() {
    let mut doc = Document::new(None);
    doc.add_column("small", DataType::Int8).unwrap();
    doc.add_row().unwrap();
    doc.set_cell(0, 0, Some("127")).unwrap();
    assert!(doc.set_cell(0, 0, Some("128")).is_err());
    assert_eq!(
        doc.frame().column("small").unwrap().dtype(),
        &DataType::Int8
    );
    assert_eq!(doc.cell(0, 0).unwrap(), Some("127".into()));
}
