// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE-BSD-3-Clause file.
//
// SPDX-License-Identifier: Apache-2.0 AND BSD-3-Clause

use std::fs::File;
use std::io::Cursor;
use std::mem::size_of;
use std::path::Path;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// These are the objects from the vm-memory branch identified as "master" in the experiments,
// and "vm-memory" in the "dev-dependencies" section of Cargo.toml. Please note that we can also
// specify a particular commit instead of a brach name in there.
use vm_memory::GuestMemoryMmap;
use vm_memory::{ByteValued, Bytes, GuestAddress};

// These are the objects from the vm-memory branch identified as "other" in the experiments,
// and "vm-memory2" in the "dev-dependencies" section of Cargo.toml. We have to alias all type
// names because Rust sees them as different from their correspondents above.
use vm_memory2::GuestMemoryMmap as GuestMemoryMmap2;
use vm_memory2::{ByteValued as ByteValued2, Bytes as Bytes2, GuestAddress as GuestAddress2};

// These are the objects from the crosvm guest memory model implementation, that were copy pasted
// in src/crosvm_mem. Right now we pretty much explicitly invoke measurement code three times
// for the three guest memory implementations under consideration. When we identify the best
// long-term benchmarking setup, we can make all of them implement `GuestMemory` and have generic
// functions/function parameters that remove the need for duplicated code. However, that's not
// possible right now because, from a Rust type system perspective, a particular version of
// the vm-memory GuestMemory interface is tied to a specific implementation, because they're
// both in the same crate. </rant>
use vm_memory_test::crosvm_mem::{
    DataInit, GuestAddress as CvmGuestAddress, GuestMemory as CvmGuestMemory,
};

use vmm_sys_util::tempfile::TempFile;

const REGION_SIZE: u64 = 0x8000_0000;
const REGIONS_COUNT: u64 = 8;
const ACCESS_SIZE: usize = 0x200;

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct SmallDummy {
    a: u32,
    b: u32,
}
unsafe impl ByteValued for SmallDummy {}
unsafe impl ByteValued2 for SmallDummy {}
unsafe impl DataInit for SmallDummy {}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct BigDummy {
    elements: [u64; 12],
}

unsafe impl ByteValued for BigDummy {}
unsafe impl ByteValued2 for BigDummy {}
unsafe impl DataInit for BigDummy {}

fn make_image(size: usize) -> Vec<u8> {
    let mut image: Vec<u8> = Vec::with_capacity(size as usize);
    for i in 0..size {
        // We just want some different numbers here, so the conversion is OK.
        image.push(i as u8);
    }
    image
}

enum AccessKind {
    // The parameter represents the index of the region where the access should happen.
    // Indices are 0-based.
    InRegion(u64),
    // Then parameter represents the index of the second region (i.e. where the access ends).
    CrossRegion(u64),
}

impl AccessKind {
    // We call this to find out if an access is cross-region, so we skip testing the crosvm
    // implementation, because it doesn't support cross-region accesses.
    fn is_cross_region(&self) -> bool {
        match self {
            AccessKind::InRegion(_) => false,
            AccessKind::CrossRegion(_) => true,
        }
    }

    fn make_offset(&self, access_size: usize) -> u64 {
        match *self {
            AccessKind::InRegion(idx) => REGION_SIZE * idx,
            AccessKind::CrossRegion(idx) => (REGION_SIZE * idx as u64) - (access_size / 2) as u64,
        }
    }
}

fn cbenchmark(c: &mut Criterion) {
    let mut regions = Vec::new();
    for i in 0..REGIONS_COUNT {
        regions.push((i * REGION_SIZE, REGION_SIZE as usize));
    }
    assert_eq!(regions.len() as u64, REGIONS_COUNT);

    let memory = GuestMemoryMmap::from_ranges(
        regions
            .iter()
            .map(|pair| (GuestAddress(pair.0), pair.1))
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .unwrap();

    let memory2 = GuestMemoryMmap2::from_ranges(
        regions
            .iter()
            .map(|pair| (GuestAddress2(pair.0), pair.1))
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .unwrap();

    let cvmem = CvmGuestMemory::new(
        regions
            .iter()
            .map(|pair| (CvmGuestAddress(pair.0), pair.1 as u64))
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .unwrap();

    let some_small_dummy = SmallDummy {
        a: 0x1111_2222,
        b: 0x3333_4444,
    };

    let some_big_dummy = BigDummy {
        elements: [0x1111_2222_3333_4444; 12],
    };

    let mut image = make_image(ACCESS_SIZE);
    let buf = &mut [0u8; ACCESS_SIZE];
    let mut file = File::open(Path::new("/dev/zero")).unwrap();
    let temp = TempFile::new().unwrap();
    let mut file_to_write = temp.as_file();

    let accesses = &[
        AccessKind::InRegion(0),
        AccessKind::CrossRegion(1),
        AccessKind::CrossRegion(REGIONS_COUNT - 1),
        AccessKind::InRegion(REGIONS_COUNT - 1),
    ];

    for access in accesses {
        let off = access.make_offset(ACCESS_SIZE);

        // Read stuff.
        {
            let mut g = c.benchmark_group(format!("read_from_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .read_from(GuestAddress(off), &mut Cursor::new(&image), ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .read_from(GuestAddress2(off), &mut Cursor::new(&image), ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            // There doesn't seem to be an equivalent method in crosvm anymore.
        }

        {
            let mut g = c.benchmark_group(format!("read_from_file_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .read_from(GuestAddress(off), &mut file, ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .read_from(GuestAddress2(off), &mut file, ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .read_to_memory(CvmGuestAddress(off), &file, ACCESS_SIZE)
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let mut g = c.benchmark_group(format!("read_exact_from_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .read_exact_from(
                                GuestAddress(off),
                                &mut Cursor::new(&mut image),
                                ACCESS_SIZE,
                            )
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .read_exact_from(
                                GuestAddress2(off),
                                &mut Cursor::new(&mut image),
                                ACCESS_SIZE,
                            )
                            .unwrap(),
                    )
                })
            });

            // There doesn't seem to be an equivalent method in crosvm anymore.
        }

        {
            let mut g = c.benchmark_group(format!("read_entire_slice_from_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| black_box(memory.read_slice(&mut buf[..], GuestAddress(off)).unwrap()))
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .read_slice(&mut buf[..], GuestAddress2(off))
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .read_exact_at_addr(&mut buf[..], CvmGuestAddress(off))
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let mut g = c.benchmark_group(format!("read_slice_from_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| black_box(memory.read(&mut buf[..], GuestAddress(off)).unwrap()))
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| black_box(memory2.read(&mut buf[..], GuestAddress2(off)).unwrap()))
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .read_at_addr(&mut buf[..], CvmGuestAddress(off))
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let obj_off = access.make_offset(size_of::<SmallDummy>());
            let mut g = c.benchmark_group(format!("read_small_obj_from_{:#0x}", obj_off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .read_obj::<SmallDummy>(GuestAddress(obj_off))
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .read_obj::<SmallDummy>(GuestAddress2(obj_off))
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .read_obj_from_addr::<SmallDummy>(CvmGuestAddress(obj_off))
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let obj_off = access.make_offset(size_of::<BigDummy>());
            let mut g = c.benchmark_group(format!("read_big_obj_from_{:#0x}", obj_off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| black_box(memory.read_obj::<BigDummy>(GuestAddress(obj_off)).unwrap()))
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .read_obj::<BigDummy>(GuestAddress2(obj_off))
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .read_obj_from_addr::<BigDummy>(CvmGuestAddress(obj_off))
                                .unwrap(),
                        )
                    })
                });
            }
        }

        // Write stuff.

        {
            let mut g = c.benchmark_group(format!("write_to_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .write_to(GuestAddress(off), &mut Cursor::new(&mut image), ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .write_to(
                                GuestAddress2(off),
                                &mut Cursor::new(&mut image),
                                ACCESS_SIZE,
                            )
                            .unwrap(),
                    )
                })
            });

            // There doesn't seem to be an equivalent method in crosvm anymore.
        }

        {
            let mut g = c.benchmark_group(format!("write_to_file_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .write_to(GuestAddress(off), &mut file_to_write, ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .write_to(GuestAddress2(off), &mut file_to_write, ACCESS_SIZE)
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .write_from_memory(CvmGuestAddress(off), file_to_write, ACCESS_SIZE)
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let mut g = c.benchmark_group(format!("write_exact_to_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .write_all_to(
                                GuestAddress(off),
                                &mut Cursor::new(&mut image),
                                ACCESS_SIZE,
                            )
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .write_all_to(
                                GuestAddress2(off),
                                &mut Cursor::new(&mut image),
                                ACCESS_SIZE,
                            )
                            .unwrap(),
                    )
                })
            });

            // There doesn't seem to be an equivalent method in crosvm anymore.
        }

        {
            let mut g = c.benchmark_group(format!("write_entire_slice_to_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| black_box(memory.write_slice(buf, GuestAddress(off)).unwrap()))
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| black_box(memory2.write_slice(buf, GuestAddress2(off)).unwrap()))
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .write_all_at_addr(&buf[..], CvmGuestAddress(off))
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let mut g = c.benchmark_group(format!("read_slice_from_{:#0x}", off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| black_box(memory.read(buf, GuestAddress(off)).unwrap()))
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| black_box(memory2.read(buf, GuestAddress2(off)).unwrap()))
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(cvmem.write_at_addr(&buf[..], CvmGuestAddress(off)).unwrap())
                    })
                });
            }
        }

        {
            let obj_off = access.make_offset(size_of::<SmallDummy>());
            let mut g = c.benchmark_group(format!("write_small_obj_to_{:#0x}", obj_off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .write_obj::<SmallDummy>(some_small_dummy, GuestAddress(obj_off))
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .write_obj::<SmallDummy>(some_small_dummy, GuestAddress2(obj_off))
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .write_obj_at_addr::<SmallDummy>(
                                    some_small_dummy,
                                    CvmGuestAddress(obj_off),
                                )
                                .unwrap(),
                        )
                    })
                });
            }
        }

        {
            let obj_off = access.make_offset(size_of::<BigDummy>());
            let mut g = c.benchmark_group(format!("write_big_obj_to_{:#0x}", obj_off).as_str());

            g.bench_function("vm-memory master", |b| {
                b.iter(|| {
                    black_box(
                        memory
                            .write_obj::<BigDummy>(some_big_dummy, GuestAddress(obj_off))
                            .unwrap(),
                    )
                })
            });

            g.bench_function("vm-memory other", |b| {
                b.iter(|| {
                    black_box(
                        memory2
                            .write_obj::<BigDummy>(some_big_dummy, GuestAddress2(obj_off))
                            .unwrap(),
                    )
                })
            });

            if !access.is_cross_region() {
                g.bench_function("crosvm", |b| {
                    b.iter(|| {
                        black_box(
                            cvmem
                                .write_obj_at_addr::<BigDummy>(
                                    some_big_dummy,
                                    CvmGuestAddress(obj_off),
                                )
                                .unwrap(),
                        )
                    })
                });
            }
        }
    }
}

criterion_group! {
    name = benches;
    // These parameters have a very large influence on the overall duration. Increasing the
    // measurement time should smooth out the outliers, but also makes the process run
    // a lot longer.
    config = Criterion::default().sample_size(200).measurement_time(std::time::Duration::from_secs(30));
    targets = cbenchmark
}

criterion_main! {
    benches,
}
