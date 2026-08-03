fn main() {
    let data = std::fs::read(".scratch/bench-datasets/csv-synthetic/data.csv").expect("read csv");
    println!("CSV size: {} bytes", data.len());

    // Whole-file Brotli
    let brotli = limnifs_core::codec::compress(0x04, &data).expect("brotli");
    println!(
        "Whole-file Brotli: {} ({:.2}%)",
        brotli.len(),
        brotli.len() as f64 / data.len() as f64 * 100.0
    );

    // Whole-file ZSTD
    let zstd = limnifs_core::codec::compress(0x02, &data).expect("zstd");
    println!(
        "Whole-file ZSTD:   {} ({:.2}%)",
        zstd.len(),
        zstd.len() as f64 / data.len() as f64 * 100.0
    );

    // FSST+Brotli (whole file)
    let fsst = limnifs_core::codec::compress(0x09, &data).expect("fsst+brotli");
    println!(
        "Whole-file FSST+B: {} ({:.2}%)",
        fsst.len(),
        fsst.len() as f64 / data.len() as f64 * 100.0
    );

    // BZip2
    let bz2 = limnifs_core::codec::compress(0x10, &data).expect("bzip2");
    println!(
        "Whole-file BZip2:  {} ({:.2}%)",
        bz2.len(),
        bz2.len() as f64 / data.len() as f64 * 100.0
    );

    // PPMd8
    let ppmd = limnifs_core::codec::compress(0x12, &data).expect("ppmd8");
    println!(
        "Whole-file PPMd8:  {} ({:.2}%)",
        ppmd.len(),
        ppmd.len() as f64 / data.len() as f64 * 100.0
    );

    // ZPAQ
    let zpaq = limnifs_core::codec::compress(0x0B, &data).expect("zpaq");
    println!(
        "Whole-file ZPAQ:   {} ({:.2}%)",
        zpaq.len(),
        zpaq.len() as f64 / data.len() as f64 * 100.0
    );
}
