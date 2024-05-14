use bevy::prelude::Vec3;
use maybe_parallel_iterator::{IntoMaybeParallelIterator, IntoMaybeParallelRefIterator};
use rand::Rng;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use voronator::{
    delaunator::{Coord, Triangulation, Vector, EPSILON},
    polygon::Polygon,
    CentroidDiagram, VoronoiDiagram,
};

#[derive(Copy, Clone, PartialEq, PartialOrd)]
struct Point {
    x: f64,
    y: f64,
}

impl Eq for Point {}

impl std::cmp::Ord for Point {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        const EPSILON: f64 = 1e-10;

        let dx = (self.x - other.x).abs();
        let dy = (self.y - other.y).abs();

        if dx < EPSILON && dy < EPSILON {
            std::cmp::Ordering::Equal
        } else if dx < dy {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }
}

impl Hash for Point {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.x.to_bits().hash(state);
        self.y.to_bits().hash(state);
    }
}

impl Coord for Point {
    fn x(&self) -> f64 {
        self.x
    }

    fn y(&self) -> f64 {
        self.y
    }

    fn from_xy(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

impl Vector<Point> for Point {
    ///
    fn vector(p: &Point, q: &Point) -> Point {
        Point::from_xy(q.x() - p.x(), q.y() - p.y())
    }

    ///
    fn determinant(p: &Point, q: &Point) -> f64 {
        p.x() * q.y() - p.y() * q.x()
    }

    ///
    fn dist2(p: &Point, q: &Point) -> f64 {
        let d = Self::vector(p, q);

        d.x() * d.x() + d.y() * d.y()
    }

    /// Test whether two coordinates describe the same point in space
    fn equals(p: &Point, q: &Point) -> bool {
        (p.x() - q.x()).abs() <= EPSILON && (p.y() - q.y()).abs() <= EPSILON
    }

    ///
    fn equals_with_span(p: &Point, q: &Point, span: f64) -> bool {
        let dist = Self::dist2(p, q) / span;
        dist < 1e-20 // dunno about this
    }
}

pub struct Heightmap {
    pub triangulation: Triangulation,
    pub heights: Vec<f64>,
    pub cells_hashmap: HashMap<Polygon<Point>, usize>,
    pub sites: Vec<Point>,
    pub sites_hashmap: HashMap<Point, usize>,
    pub cells: Vec<Polygon<Point>>,
    pub cell_neighbors: Vec<Vec<usize>>,
    pub seed_points: Vec<Point>,
    pub plate_vectors: Vec<Vec3>,
}

impl Heightmap {
    pub fn new(diagram: VoronoiDiagram<Point>, seed_points: Option<Vec<Point>>) -> Self {
        let mut seeds;
        let cell_hashmap: HashMap<Polygon<Point>, usize> = diagram
            .cells()
            .iter()
            .enumerate()
            .map(|(i, cell)| (cell.clone(), i))
            .collect();

        if let Some(points) = seed_points {
            seeds = points;
        } else {
            let mut rng = rand::thread_rng();
            let mut rand_num;
            seeds = Vec::new();
            for i in 0..5 {
                rand_num = rng.gen_range(0..(diagram.sites.len() / 4)) * (i + 1);
                seeds.push(diagram.sites[rand_num]);
            }
        }
        Self {
            triangulation: diagram.delaunay.clone(),
            heights: vec![0.0; diagram.cells().len()],
            cells: diagram.cells().into(),
            cell_neighbors: diagram.neighbors,
            cells_hashmap: cell_hashmap,
            plate_vectors: vec![Vec3::new(0.0, 0.0, 0.0); seeds.clone().len()],
            seed_points: seeds,
            sites: diagram.sites.clone(),
            sites_hashmap: diagram
                .sites
                .iter()
                .enumerate()
                .map(|(i, site)| (site.clone(), i))
                .collect(),
        }
    }
    pub fn generate_heights(&mut self, plate_vectors: Vec<Vec3>) {
        self.plate_vectors = plate_vectors;
        let visited: HashMap<Point, bool> = HashMap::new();
        for seed in &self.seed_points {
            let mut queue: VecDeque<Point> = VecDeque::new();
        }
    }
}
