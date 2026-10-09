import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import install_qualified_build as installer


class InstallDestinations(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.build = self.root / "build"
        self.build.mkdir()
        self.binary = self.build / "qa-rust"
        self.binary.write_bytes(b"new normal candidate")
        self.binary.chmod(0o755)
        self.metadata = dict(commit="test-commit", source_tree_dirty=False,
                             proof=False, target_cpu="baseline", build_time_seconds=1.25,
                             built_at_utc="2026-10-09T00:00:00Z")
        self.save_metadata()
        self.profile = self.root / "profile"
        self.profile.mkdir()
        (self.profile / "owner.cfg").write_bytes(b"saved settings")
        self.destination = self.root / "qfiles"
        self.destination.mkdir()
        self.regular = self.destination / "qa-rust"
        self.regular.write_bytes(b"qualified original")
        self.original_identity = installer.identity(self.regular)
        self.calls = []
        self.gameplay = False
        self.smoke = False
        self.reject_map = None
        self.run_patch = patch.object(installer, "run", side_effect=self.private_result)
        self.run_patch.start()
        self.addCleanup(self.run_patch.stop)
        self.info_patch = patch.object(installer.subprocess, "check_output",
                                       side_effect=lambda *args, **kwargs: json.dumps(self.metadata))
        self.info_patch.start()
        self.addCleanup(self.info_patch.stop)

    def save_metadata(self):
        (self.build / "build.json").write_text(json.dumps(self.metadata))

    def private_result(self, binary, profile, evidence, arguments, audio_driver="disk", timeout=30):
        self.assertEqual(audio_driver, "dummy" if self.smoke else "disk")
        self.assertEqual(timeout, 60 if self.smoke else 30)
        self.calls.append(arguments)
        name = arguments[arguments.index("--map") + 1]
        renderer = arguments[arguments.index("--renderer") + 1]
        result = dict(result="PASS", normal_exit=True, copied_owner_settings=["owner.cfg"],
                      owner_profile_unchanged=True, candidate_unchanged=True,
                      remaining_owned_pids=[], gameplay_reached=self.gameplay,
                      candidate_identity=installer.identity(binary), events=[
                          dict(event="world_frame_presented", map="maps/" + name + ".bsp",
                               renderer=renderer, client_connected=True,
                               views=0 if name == self.reject_map else 1, surfaces=10,
                               rejected=18 if name == "start" and renderer == "cpu" else 0,
                               profile_consumed=True, native_input_policy=True),
                          dict(event="normal_exit", frames=300)])
        evidence.mkdir(parents=True)
        (evidence / "result.json").write_text(json.dumps(result))
        return result

    def install(self, destination, arguments=None, smoke=False):
        self.smoke = smoke
        return installer.install(self.build, destination, self.profile, self.root / "evidence",
                                 arguments or [], owner_smoke=smoke)

    def regular_unchanged(self):
        self.assertEqual(self.regular.read_bytes(), b"qualified original")
        self.assertEqual(installer.identity(self.regular), self.original_identity)
        self.assertEqual((self.profile / "owner.cfg").read_bytes(), b"saved settings")

    def test_owner_smoke_replaces_regular_binary_after_six_runs_and_writes_gaps(self):
        receipt = self.install(self.regular, smoke=True)
        self.assertEqual(len(self.calls), 6)
        self.assertEqual({(args[args.index("--map") + 1], args[args.index("--renderer") + 1])
                          for args in self.calls},
                         {(name, renderer) for _, name in installer.SMOKE_MAPS for renderer in ("gl", "cpu")})
        self.assertEqual(self.regular.read_bytes(), self.binary.read_bytes())
        self.assertEqual(receipt["qualification_scope"], "render_smoke")
        self.assertFalse(receipt["gameplay_qualified"])
        self.assertEqual(receipt["performance"]["result"], "NOT_QUALIFIED")
        self.assertIn(self.metadata["commit"], self.regular.with_suffix(".txt").read_text())
        notes = self.regular.with_suffix(".txt").read_text()
        for gap in ("native hosts", "delta channel", "stock HUD"):
            self.assertIn(gap, notes)
        self.assertIn("start cpu: 10 surfaces presented, 18 rejected", notes)
        self.assertEqual((self.profile / "owner.cfg").read_bytes(), b"saved settings")

    def test_regular_still_rejects_missing_gameplay_marker(self):
        with self.assertRaises(ValueError):
            self.install(self.regular, ["--map", "base1", "--renderer", "gl"])
        self.regular_unchanged()

    def test_regular_still_requires_gameplay_timings_after_marker(self):
        self.gameplay = True
        with self.assertRaises(ValueError):
            self.install(self.regular, ["--map", "base1", "--renderer", "gl"])
        self.regular_unchanged()

    def test_smoke_rejects_missing_world_without_replacing_old_files(self):
        self.reject_map = "base1"
        notes = self.regular.with_suffix(".txt")
        notes.write_bytes(b"previous notes")
        with self.assertRaises(ValueError):
            self.install(self.regular, smoke=True)
        self.assertEqual(self.regular.read_bytes(), b"qualified original")
        self.assertEqual(notes.read_bytes(), b"previous notes")
        self.regular_unchanged()

    def test_smoke_rejects_proof_or_dirty_candidates_before_running(self):
        for field in ("proof", "source_tree_dirty"):
            with self.subTest(field=field):
                self.metadata[field] = True
                self.save_metadata()
                with self.assertRaises(ValueError):
                    installer.qualify(self.build, self.profile, self.root / field, [], require_gameplay=False)
                self.metadata[field] = False
        self.assertEqual(self.calls, [])
        self.regular_unchanged()

    def test_other_destination_names_cannot_use_the_smoke_exception(self):
        with self.assertRaises(ValueError):
            self.install(self.destination / "other-build", smoke=True)
        self.assertEqual(self.calls, [])
        self.regular_unchanged()


if __name__ == "__main__":
    unittest.main()
