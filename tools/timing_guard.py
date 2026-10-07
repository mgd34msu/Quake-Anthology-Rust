"""Compare measured gameplay workloads before an install replaces the binary."""
import math

STAGES = ("sim", "scene", "draw", "present", "audio", "total")


def rows(report):
    if report.get("scope") != "gameplay" or report.get("measured") is not True:
        raise ValueError("installation requires measured gameplay timings")
    indexed = {}
    for row in report["rows"]:
        if not row.get("map") or not isinstance(row.get("workload"), dict) or not row["workload"]:
            raise ValueError("timing row must name its map and reproducible workload")
        if not {"roles", "settings", "seed", "simulation_steps", "final_state", "events"} <= row["workload"].keys():
            raise ValueError("timing workload lacks settings, roles, seed, simulation steps or fidelity outputs")
        if (row.get("frames") != 600 or row.get("warmup") != 60
                or row.get("debugger") is not False or row.get("vsync") is not False):
            raise ValueError("timing row violates the measurement protocol")
        key = (row["map"], row["renderer"], row["resolution"])
        if key in indexed:
            raise ValueError("duplicate timing workload")
        for stage in STAGES:
            for metric in ("median_ms", "p99_ms"):
                value = row["stages"][stage][metric]
                if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                    raise ValueError("invalid measured frame time")
        indexed[key] = row
    if not indexed:
        raise ValueError("no measured workloads")
    return indexed


def compare(candidate, baseline, build, artifact_identity):
    if (candidate.get("commit") != build["commit"]
            or candidate.get("artifact_identity") != artifact_identity):
        raise ValueError("timing evidence belongs to another candidate")
    for field in ("target_cpu", "machine"):
        if not candidate.get(field) or candidate[field] != baseline.get(field):
            raise ValueError("timing baseline is not comparable: " + field)
    if candidate["target_cpu"] != build["target_cpu"]:
        raise ValueError("timing CPU target differs from the build")
    new, old = rows(candidate), rows(baseline)
    if new.keys() != old.keys():
        raise ValueError("timing workload sets differ")
    checked = []
    for key, row in new.items():
        previous = old[key]
        if row["workload"] != previous["workload"] or not row.get("cores") or row["cores"] != previous.get("cores"):
            raise ValueError("timing workload or pinned cores differ")
        for stage in STAGES:
            for metric in ("median_ms", "p99_ms"):
                value = row["stages"][stage][metric]
                reference = previous["stages"][stage][metric]
                if value > reference * 1.1:
                    raise ValueError(f"performance regression over 10%: {key} {stage} {metric}")
        checked.append({"map": key[0], "renderer": key[1], "resolution": key[2]})
    return {"result": "PASS", "maximum_regression_percent": 10, "workloads": checked,
            "baseline_commit": baseline["commit"]}
