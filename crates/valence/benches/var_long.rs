use std::hint::black_box;

use criterion::Criterion;
use rand::Rng;
use valence::packet::var_long::VarLong;
use valence::packet::{Decode, Encode};

pub fn var_long(c: &mut Criterion) {
    let mut rng = rand::thread_rng();

    c.bench_function("VarLong::encode", |b| {
        b.iter_with_setup(
            || rng.gen(),
            |i| {
                let i: i64 = black_box(i);

                let mut buf = [0; VarLong::MAX_SIZE];
                let _ = black_box(VarLong(i).encode(buf.as_mut_slice()));
            },
        );
    });

    c.bench_function("VarLong::decode", |b| {
        b.iter_with_setup(
            || {
                let mut buf = [0; VarLong::MAX_SIZE];
                VarLong(rng.gen()).encode(buf.as_mut_slice()).unwrap();
                buf
            },
            |buf| {
                let mut r = black_box(buf.as_slice());
                let _ = black_box(VarLong::decode(&mut r));
            },
        )
    });
}
