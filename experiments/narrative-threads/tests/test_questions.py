from questions import locate, shares, windows


def test_locate_exact_bytes_only():
    data = "alpha beta. Gamma délta.".encode()
    s = locate(data, 0, len(data), "Gamma délta.")
    assert data[s[0]:s[1]] == "Gamma délta.".encode()
    assert locate(data, 0, len(data), "gamma delta") is None
    assert locate(data, 0, len(data), "  ") is None


def test_windows_cover_blocks_within_size():
    data = b"\n\n".join(b"x" * 40 for _ in range(10))
    ws = windows(data, size=100)
    assert ws[0][0] == 0 and ws[-1][1] == len(data)
    assert all(e - s <= 100 for s, e in ws)
    assert all(a[1] < b[0] for a, b in zip(ws, ws[1:]))


def test_shares_sum():
    assert shares(30, 5) == [6] * 5
    assert sum(shares(30, 4)) == 30


def test_locate_tolerates_line_wraps_not_word_changes():
    data = b"He said that\r\nthe whale was white."
    s = locate(data, 0, len(data), "that the whale")
    assert data[s[0]:s[1]] == b"that\r\nthe whale"
    assert locate(data, 0, len(data), "that a whale") is None


def test_model_calls_are_isolated_from_hooks_and_settings():
    from questions import isolated_call
    argv, env = isolated_call()
    assert env["LOCKS_HOOKS_OFF"] == "1"
    assert argv[argv.index("--setting-sources") + 1] == ""
    assert argv[argv.index("--tools") + 1] == ""
