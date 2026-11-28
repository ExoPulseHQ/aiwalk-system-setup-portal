"""Checks `exo train`: the preflight decision, reading a training yaml with and without PyYAML, and the run states
`watch` reports. No GPU, SCONE or systemd needed. Run: python3 hosts/test_exo_train.py"""
import builtins, os, tempfile, time
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
exo = SourceFileLoader("exo", f"{HERE}/exo").load_module()
MEM = {"MemTotal": 31.3, "MemAvailable": 25.0}


def check(mem=MEM, gpu=((0, 7.0, 8.0),), load=1.0, ncpu=16, runs=1, gb=7.3, gpu_gb=1.0, cpus=6, margin=4, pending=0.0, strict=False):
    return exo.check(mem, list(gpu), load, ncpu, runs, gb, gpu_gb, cpus, margin, pending, strict)


# fits: two runs on a mostly free machine
ok, reasons, warnings, _ = check(runs=2)
assert ok and not reasons and not warnings, reasons
# horse at 07:46 on 2026-11-27: 1 GB available while two runs were going; a third is refused
ok, reasons, _, _ = check(mem={"MemTotal": 31.3, "MemAvailable": 1.0}, load=5.0)
assert not ok and reasons[0].startswith("RAM"), reasons
# a seed loop: the run started a minute ago will still take 6 GB
assert check(mem={"MemTotal": 31.3, "MemAvailable": 15.0}, gb=6.2)[0]
ok, reasons, _, _ = check(mem={"MemTotal": 31.3, "MemAvailable": 15.0}, gb=6.2, pending=6.0)
assert not ok and "still to come" in reasons[0], reasons
# more than half the RAM is refused even when it is free
ok, reasons, _, _ = check(mem={"MemTotal": 31.3, "MemAvailable": 30.0}, gpu=(), runs=3, cpus=4)
assert not ok and "half" in reasons[0], reasons
# parallel: 20 on 16 CPUs: a warning, refused only with --strict-cpu
ok, reasons, warnings, _ = check(gb=5.0, cpus=21)
assert ok and not reasons and warnings[0].startswith("CPU"), (reasons, warnings)
ok, reasons, _, _ = check(gb=5.0, cpus=21, strict=True)
assert not ok and reasons[0].startswith("CPU"), reasons
# GPU memory
ok, reasons, _, _ = check(gpu=((0, 0.5, 16.0),), gb=5)
assert not ok and reasons[0].startswith("GPU"), reasons

with tempfile.TemporaryDirectory() as t:
    # the training yaml, with PyYAML and with the line reader a python without it falls back to
    cfg = f"{t}/c.yaml"
    open(cfg, "w").write("tonic:\n  name: 'h2190_walk s0'\n  parallel: 5   # envs\n  sequential: 40\nenv_args:\n  parallel: 99\n")
    real_import = builtins.__import__
    def no_yaml(name, *a, **k):
        if name == "yaml":
            raise ImportError(name)
        return real_import(name, *a, **k)
    builtins.__import__ = no_yaml
    try:
        assert exo.tonic_config(cfg) == (5, "h2190_walk s0"), exo.tonic_config(cfg)
    finally:
        builtins.__import__ = real_import
    try:
        import yaml  # noqa: F401
        assert exo.tonic_config(cfg) == (5, "h2190_walk s0"), exo.tonic_config(cfg)
    except ImportError:
        pass
    gb, cpus = exo.per_run_from_config(cfg)
    assert abs(gb - (exo.MAIN_GB + 5 * exo.WORKER_GB)) < 1e-9 and cpus == 6

    # run states
    trainer = "deprl.custom_trainer.Trainer(steps=int(2e7), epoch_steps=int(2e5), save_steps=int(1e6))"
    def run_dir(base, steps, trainer, age_min):
        d = f"{t}/{base}/myrun/261127.000000.ModelHexis"
        os.makedirs(d)
        open(f"{d}/log.csv", "w").write(f"train/steps,x\n{steps},1\n")
        open(f"{d}/config.yaml", "w").write(f"  trainer: {trainer}\n")
        old = time.time() - age_min * 60
        os.utime(f"{d}/log.csv", (old, old))
        return f"{t}/{base}/myrun"
    assert exo.run_status(run_dir("a", 1e6, trainer, 5), 30)["state"] == "running"
    assert exo.run_status(run_dir("b", 1e6, trainer, 45), 30)["state"] == "stale"
    assert exo.run_status(run_dir("c", 2e7, trainer, 300), 30)["state"] == "finished"
    assert exo.run_status(f"{t}/missing", 30)["state"] == "no-log"
    os.makedirs(f"{t}/young")
    assert exo.run_status(f"{t}/young", 30, grace_min=20)["state"] == "starting"
    # steps= is the total, not epoch_steps=
    st = exo.run_status(run_dir("d", 4e5, "Trainer(epoch_steps=int(2e5), steps=int(2e7))", 45), 30)
    assert st["total"] == 2e7 and st["state"] == "stale", st

# stop touches only the scopes launch made
try:
    exo.stop("gnome-terminal.scope"); raise AssertionError("stopped a scope launch did not make")
except exo.Refuse:
    pass
print("exo train: ok")
