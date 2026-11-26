import kerno
import pytest


def test_buffered_durability_keeps_commits_across_close(tmp_path):
    db = kerno.open(tmp_path / "db", durability="buffered")
    ids = db.commit([("store", "note", b"a"), ("store", "note", b"b")])
    db.close()
    db = kerno.open(tmp_path / "db")
    assert [db.read(i).payload for i in ids] == [b"a", b"b"]
    db.close()


def test_unknown_durability_is_refused(tmp_path):
    with pytest.raises(kerno.InvalidArgumentError):
        kerno.open(tmp_path / "db", durability="fast")


def test_close_without_compaction_keeps_every_unit(tmp_path):
    import kerno

    path = str(tmp_path / "db")
    db = kerno.open(path, close_compaction=False)
    ids = [db.commit([("store", "note", f"n{i}")])[0] for i in range(5)]
    db.close()
    db = kerno.open(path)
    assert [db.read(i).payload for i in ids] == [f"n{i}".encode() for i in range(5)]
    db.close()
