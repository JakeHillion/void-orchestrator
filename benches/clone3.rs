use clone_shim::clone::{clone3, CloneArgs, CloneFlags};

use criterion::{criterion_group, criterion_main, Criterion};

fn run_clone3(flags: CloneFlags) {
    if clone3(CloneArgs::new(flags)).unwrap() == nix::unistd::Pid::from_raw(0) {
        std::process::exit(0)
    }
}

pub fn benchmark_clone3(c: &mut Criterion) {
    c.bench_function("clone3", |b| b.iter(|| run_clone3(CloneFlags::empty())));
    c.bench_function("clone3+CLONE_NEWCGROUP", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWCGROUP))
    });
    c.bench_function("clone3+CLONE_NEWIPC", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWIPC))
    });
    c.bench_function("clone3+CLONE_NEWNET", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWNET))
    });
    c.bench_function("clone3+CLONE_NEWNS", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWNS))
    });
    c.bench_function("clone3+CLONE_NEWPID", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWPID))
    });
    c.bench_function("clone3+CLONE_NEWUSER", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWUSER))
    });
    c.bench_function("clone3+CLONE_NEWUTS", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWUTS))
    });
    c.bench_function("clone3+CLONE_NEWCGROUP+CLONE_NEWIPC+CLONE_NEWNET+CLONE_NEWNS+CLONE_NEWPID+CLONE_NEWUSER+CLONE_NEWUTS", |b| {
        b.iter(|| run_clone3(CloneFlags::CLONE_NEWCGROUP|CloneFlags::CLONE_NEWIPC|CloneFlags::CLONE_NEWNET|CloneFlags::CLONE_NEWNS|CloneFlags::CLONE_NEWPID|CloneFlags::CLONE_NEWUSER|CloneFlags::CLONE_NEWUTS))
    });
}

criterion_group!(benches, benchmark_clone3);
criterion_main!(benches);
