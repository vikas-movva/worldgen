use worldgen_core::mesh;
fn main() {
    for n in [4u32, 5, 6, 10, 20, 50, 100] {
        let m = mesh::build(n, 42);
        let nv = m.points.len();
        let mut minv = 100usize;
        let mut bad = 0;
        for c in 0..nv {
            let lo = m.cells.i[c] as usize;
            let hi = m.cells.i[c+1] as usize;
            let k = hi - lo;
            if k < minv { minv = k; }
            if k < 3 { bad += 1; }
        }
        let border = m.cells.b.iter().filter(|&&b| b == 1).count();
        println!("n={} -> cells={} minVerts={} bad={} border={}", n, nv, minv, bad, border);
    }
}
