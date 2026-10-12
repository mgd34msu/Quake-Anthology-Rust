import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import install_qualified_build as installer
from timing_guard import STAGES


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

    def private_result(self, binary, profile, evidence, arguments, audio_driver="disk", timeout=30,
                       actions=None, input_after_first_frame=False):
        self.assertEqual(audio_driver, "dummy" if self.smoke else "disk")
        self.assertEqual(timeout, 60 if self.smoke else 30)
        self.assertEqual(input_after_first_frame, bool(actions))
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

    def install(self, destination, arguments=None, smoke=False, timings=None, baseline=None):
        self.smoke = smoke
        return installer.install(self.build, destination, self.profile, self.root / "evidence",
                                 arguments or [], timings=timings, baseline=baseline, owner_smoke=smoke)

    def gameplay_timings(self):
        self.gameplay = True
        workload = dict(roles={"movement": "q2"}, settings={"r_swapInterval": 0},
                        seed=17, simulation_steps=600, final_state={"position": [1, 2, 3]}, events=[])
        row = dict(map="base1", renderer="gl", resolution="640x400", frames=600, warmup=60,
                   debugger=False, vsync=False, workload=workload, cores=[23],
                   stages={stage: dict(median_ms=1.0, p99_ms=2.0) for stage in STAGES})
        report = dict(scope="gameplay", measured=True, commit=self.metadata["commit"],
                      artifact_identity=installer.identity(self.binary), target_cpu="baseline",
                      machine="isolated test fixture", rows=[row])
        timings = self.root / "timings.json"
        timings.write_text(json.dumps(report))
        baseline = self.root / "baseline.json"
        baseline.write_text(json.dumps(dict(report, commit="prior-test-commit")))
        return timings, baseline

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

    def test_regular_refreshes_companion_notes_after_gameplay_and_timing_qualification(self):
        timings, baseline = self.gameplay_timings()
        notes = self.regular.with_suffix(".txt")
        notes.write_text("obsolete development build")
        receipt = self.install(self.regular, ["--map", "base1", "--renderer", "gl"],
                               timings=timings, baseline=baseline)
        self.assertEqual(self.regular.read_bytes(), self.binary.read_bytes())
        self.assertEqual(receipt["qualification_scope"], "gameplay")
        self.assertTrue(receipt["gameplay_qualified"])
        self.assertEqual(receipt["performance"]["result"], "PASS")
        text = notes.read_text()
        self.assertIn(self.metadata["commit"], text)
        self.assertIn(self.metadata["built_at_utc"], text)
        self.assertIn("Qualification scope: gameplay and comparable gameplay timings", text)
        self.assertIn("prior-test-commit", text)
        self.assertIn("base1 gl 640x400", text)
        self.assertNotIn("owner-authorized render smoke", text)
        self.assertNotIn("obsolete development build", text)
        self.assertEqual((self.profile / "owner.cfg").read_bytes(), b"saved settings")

    def test_regular_timing_regression_preserves_previous_install_and_notes(self):
        timings, baseline = self.gameplay_timings()
        report = json.loads(timings.read_text())
        report["rows"][0]["stages"]["draw"]["median_ms"] = 2.0
        timings.write_text(json.dumps(report))
        notes = self.regular.with_suffix(".txt")
        notes.write_bytes(b"previous notes")
        with self.assertRaises(ValueError):
            self.install(self.regular, ["--map", "base1", "--renderer", "gl"],
                         timings=timings, baseline=baseline)
        self.regular_unchanged()
        self.assertEqual(notes.read_bytes(), b"previous notes")

    def assert_note_staging_failure_preserves_install(self, smoke):
        notes = self.regular.with_suffix(".txt")
        notes.write_bytes(b"previous notes")
        timings, baseline = (None, None) if smoke else self.gameplay_timings()
        arguments = [] if smoke else ["--map", "base1", "--renderer", "gl"]
        with patch.object(installer, "stage_text", side_effect=OSError("staging failed")):
            with self.assertRaises(OSError):
                self.install(self.regular, arguments, smoke=smoke, timings=timings, baseline=baseline)
        self.regular_unchanged()
        self.assertEqual(notes.read_bytes(), b"previous notes")
        self.assertEqual(sorted(path.name for path in self.destination.iterdir()), ["qa-rust", "qa-rust.txt"])

    def test_regular_note_staging_failure_preserves_previous_install(self):
        self.assert_note_staging_failure_preserves_install(False)

    def test_smoke_note_staging_failure_preserves_previous_install(self):
        self.assert_note_staging_failure_preserves_install(True)

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
