"""
End-to-end tests for XetRangeUploadCommit using a local CAS endpoint.

Run after building the extension:
    cd hf_xet && maturin develop
    pytest tests/test_range_upload.py -v --tb=short

These tests use a local CAS endpoint (no HF token or bucket required).
"""

import hf_xet


# ── Helpers ──────────────────────────────────────────────────────────────────

def _upload_bytes(session: "hf_xet.XetSession", endpoint: str, data: bytes) -> "hf_xet.XetFileInfo":
    """Upload raw bytes via the regular upload commit and return XetFileInfo."""
    commit = session.new_upload_commit(endpoint=endpoint)
    h = commit.start_upload_bytes(data, sha256=hf_xet.SKIP_SHA256)
    commit.wait_to_finish()
    return h.result().xet_info


def _download_via_group(session: "hf_xet.XetSession", endpoint: str, file_info: "hf_xet.XetFileInfo", dest_path: str):
    """Download a file via the file download group and return the file info."""
    group = session.new_file_download_group(endpoint=endpoint)
    h = group.start_download_file(file_info, dest_path)
    report = group.wait_to_finish()
    return report.downloads[h.task_id()].file_info


# ── Edit (replace bytes) ────────────────────────────────────────────────────

class TestRangeUploadEdit:
    """Test: upload original, edit bytes 0..13, verify composed result."""

    def test_e2e_range_upload_edit(self, endpoint, tmp_path):
        original_data = b"Hello, World! This is a test file for range upload."
        assert len(original_data) == 51

        session = hf_xet.XetSession()

        # Step 1: upload original
        original_info = _upload_bytes(session, endpoint, original_data)
        assert original_info.file_size == 51

        # Step 2: edit bytes 0..13 ("Hello, World!") -> "Universe! " (10 bytes)
        # Expected new size: 51 - 13 + 10 = 48
        commit = session.new_range_upload(original_info.hash, original_info.file_size, endpoint=endpoint)

        edit = commit.edit((0, 13), 10)
        edit.write(b"Universe! ")
        # No need to call finish() explicitly — commit handles it

        report = commit.commit()
        assert report.file_info.file_size == 48

        # Step 3: download and verify content
        dest_path = tmp_path / "range_upload_edit_test.txt"
        _download_via_group(session, endpoint, report.file_info, str(dest_path))
        content = dest_path.read_bytes()
        assert content == b"Universe!  This is a test file for range upload."


# ── Insert ──────────────────────────────────────────────────────────────────

class TestRangeUploadInsert:
    """Test: upload original, insert bytes at position, verify result."""

    def test_e2e_range_upload_insert(self, endpoint, tmp_path):
        original_data = b"ABCDEF"
        assert len(original_data) == 6

        session = hf_xet.XetSession()

        # Step 1: upload original
        original_info = _upload_bytes(session, endpoint, original_data)
        assert original_info.file_size == 6

        # Step 2: insert "XYZ" at position 2 (between B and C)
        # Expected new size: 6 + 3 = 9
        commit = session.new_range_upload(original_info.hash, original_info.file_size, endpoint=endpoint)

        edit = commit.insert(2, 3)
        edit.write(b"XYZ")
        # No need to call finish() explicitly — commit handles it

        report = commit.commit()
        assert report.file_info.file_size == 9

        # Step 3: download and verify content
        dest_path = tmp_path / "range_upload_insert_test.txt"
        _download_via_group(session, endpoint, report.file_info, str(dest_path))
        content = dest_path.read_bytes()
        assert content == b"ABXYZCDEF"


# ── Delete ──────────────────────────────────────────────────────────────────

class TestRangeUploadDelete:
    """Test: upload original, delete bytes, verify result."""

    def test_e2e_range_upload_delete(self, endpoint, tmp_path):
        original_data = b"Hello, World!"
        assert len(original_data) == 13

        session = hf_xet.XetSession()

        # Step 1: upload original
        original_info = _upload_bytes(session, endpoint, original_data)
        assert original_info.file_size == 13

        # Step 2: delete bytes 5..12 (", World") — 7 bytes removed
        # Expected new size: 13 - 7 = 6
        commit = session.new_range_upload(original_info.hash, original_info.file_size, endpoint=endpoint)

        edit = commit.delete(5, 12)
        # No need to call finish() — delete edits have no data to write

        report = commit.commit()
        assert report.file_info.file_size == 6

        # Step 3: download and verify content
        dest_path = tmp_path / "range_upload_delete_test.txt"
        _download_via_group(session, endpoint, report.file_info, str(dest_path))
        content = dest_path.read_bytes()
        assert content == b"Hello!"


# ── Append ──────────────────────────────────────────────────────────────────

class TestRangeUploadAppend:
    """Test: upload original, append bytes at end, verify result."""

    def test_e2e_range_upload_append(self, endpoint, tmp_path):
        original_data = b"Hello, "
        assert len(original_data) == 7

        session = hf_xet.XetSession()

        # Step 1: upload original
        original_info = _upload_bytes(session, endpoint, original_data)
        assert original_info.file_size == 7

        # Step 2: append "World!" (6 bytes) at end
        # Expected new size: 7 + 6 = 13
        commit = session.new_range_upload(original_info.hash, original_info.file_size, endpoint=endpoint)

        edit = commit.append(6)
        edit.write(b"World!")
        # No need to call finish() explicitly — commit handles it

        report = commit.commit()
        assert report.file_info.file_size == 13

        # Step 3: download and verify content
        dest_path = tmp_path / "range_upload_append_test.txt"
        _download_via_group(session, endpoint, report.file_info, str(dest_path))
        content = dest_path.read_bytes()
        assert content == b"Hello, World!"


# ── Multiple edits ──────────────────────────────────────────────────────────

class TestRangeUploadMultipleEdits:
    """Test: upload original, apply multiple edits in one commit."""

    def test_e2e_range_upload_multiple_edits(self, endpoint, tmp_path):
        original_data = b"0123456789ABCDEF"  # 16 bytes
        session = hf_xet.XetSession()

        # Step 1: upload original
        original_info = _upload_bytes(session, endpoint, original_data)
        assert original_info.file_size == 16

        # Step 2: apply multiple edits
        # - edit 0..4 ("0123" -> "XXXX") - keep same length
        # - insert 8, 3 ("---") - 3 bytes added
        # - delete 14..16 ("EF") — 2 bytes removed
        # Expected: "XXXX4567---89ABCD" = 17 bytes
        commit = session.new_range_upload(original_info.hash, original_info.file_size, endpoint=endpoint)

        edit1 = commit.edit((0, 4), 4)
        edit1.write(b"XXXX")

        edit2 = commit.insert(8, 3)
        edit2.write(b"---")

        edit3 = commit.delete(14, 16)

        report = commit.commit()
        assert report.file_info.file_size == 17

        # Step 3: download and verify
        dest_path = tmp_path / "range_upload_multi_test.txt"
        _download_via_group(session, endpoint, report.file_info, str(dest_path))
        content = dest_path.read_bytes()
        assert content == b"XXXX4567---89ABCD"


# ── Successive edits ────────────────────────────────────────────────────────

class TestRangeUploadSuccessiveEdits:
    """Test: apply multiple range edits sequentially, each built on the previous result."""

    def test_e2e_range_upload_successive_appends(self, endpoint, tmp_path):
        """Test two successive append operations."""
        original_data = b"Hello, "
        session = hf_xet.XetSession()

        # Step 1: upload original
        info = _upload_bytes(session, endpoint, original_data)
        assert info.file_size == 7

        # Step 2: first append — "World"
        commit1 = session.new_range_upload(info.hash, info.file_size, endpoint=endpoint)
        edit1 = commit1.append(5)
        edit1.write(b"World")
        report1 = commit1.commit()
        assert report1.file_info.file_size == 12

        dest1 = tmp_path / "step1.txt"
        _download_via_group(session, endpoint, report1.file_info, str(dest1))
        assert dest1.read_bytes() == b"Hello, World"

        # Step 3: second append on the new file — "!"
        commit2 = session.new_range_upload(
            report1.file_info.hash, report1.file_info.file_size, endpoint=endpoint
        )
        edit2 = commit2.append(1)
        edit2.write(b"!")
        report2 = commit2.commit()
        assert report2.file_info.file_size == 13

        dest2 = tmp_path / "step2.txt"
        _download_via_group(session, endpoint, report2.file_info, str(dest2))
        content = dest2.read_bytes()
        assert content == b"Hello, World!"

    def test_e2e_range_upload_edit_then_append(self, endpoint, tmp_path):
        """Test: edit then append — mixed operations across successive commits."""
        original_data = b"Hello, World!"
        session = hf_xet.XetSession()

        # Step 1: upload original
        info = _upload_bytes(session, endpoint, original_data)
        assert info.file_size == 13

        # Step 2: edit "World" -> "Universe" at position 7 (7 -> 8 bytes, +3 total)
        # "World" is 5 bytes (positions 7..12), replacing with 8-byte "Universe"
        # Expected: 13 - 5 + 8 = 16
        commit1 = session.new_range_upload(info.hash, info.file_size, endpoint=endpoint)
        edit1 = commit1.edit((7, 12), 8)
        edit1.write(b"Universe")
        report1 = commit1.commit()
        assert report1.file_info.file_size == 16

        dest1 = tmp_path / "step1.txt"
        _download_via_group(session, endpoint, report1.file_info, str(dest1))
        assert dest1.read_bytes() == b"Hello, Universe!"

        # Step 3: append " again" at end (6 bytes added)
        commit2 = session.new_range_upload(
            report1.file_info.hash, report1.file_info.file_size, endpoint=endpoint
        )
        edit2 = commit2.append(6)
        edit2.write(b" again")
        report2 = commit2.commit()
        assert report2.file_info.file_size == 22

        dest2 = tmp_path / "step2.txt"
        _download_via_group(session, endpoint, report2.file_info, str(dest2))
        content = dest2.read_bytes()
        assert content == b"Hello, Universe! again"
