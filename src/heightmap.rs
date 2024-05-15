use bevy::prelude::Vec3;
use maybe_parallel_iterator::{IntoMaybeParallelIterator, IntoMaybeParallelRefIterator};
use rand::distributions::Distribution;
use rand::Rng;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use voronator::{
    delaunator::{Coord, Triangulation, Vector, EPSILON},
    polygon::Polygon,
    CentroidDiagram, VoronoiDiagram,
};

#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub struct Point {
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
    pub cell_neighbors: Vec<Vec<usize>>, // list of indices of neighboring cells
    pub seed_points: Vec<Point>,
    pub plate_vectors: Vec<Vec3>,
}

impl Heightmap {
    pub fn new(diagram: VoronoiDiagram<Point>, seed_points: Option<Vec<Point>>, plate_vectors: Option<Vec<Vec3>>) -> Self {
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
            let num = diagram.sites.len() / 6;
            for i in 0..5 {
                rand_num = rng.gen_range((num*i)..(num*(i+1)));
                seeds.push(diagram.sites[rand_num]);
            }
        }
        Self {
            triangulation: diagram.delaunay.clone(),
            heights: vec![0.0; diagram.cells().len()],
            cells: diagram.cells().into(),
            cell_neighbors: diagram.neighbors,
            cells_hashmap: cell_hashmap,
            plate_vectors: plate_vectors.unwrap_or(vec![Vec3::new(0.0, 0.0, 0.0); seeds.clone().len()]),
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
    pub fn generate_heights(&mut self) {
        if self.plate_vectors == vec![Vec3::new(0.0, 0.0, 0.0); self.seed_points.len()] {
            self.generate_plate_vectors();
        }

        let visited: HashMap<usize, bool> = HashMap::new();
        for seed in &self.seed_points {
            let mut queue: VecDeque<usize> = VecDeque::new();
            let index = self.sites_hashmap.get(seed).unwrap();
            let adjacent_cells = &self.cell_neighbors[*index];
            queue.push_back(*index);
            for cell in adjacent_cells {
                queue.push_back(*cell);
            }
            const P_HEIGHT: f64 = 0.75;
            const P_STOP: f64 = 0.01;
            let mut rng = rand::thread_rng();
            let distribution = rand::distributions::Uniform::new(0.0, 1.0);

            let mut height = 0.9;
            let mut stop = false;

            while !queue.is_empty() {
                let current_cell = queue.pop_front().unwrap();
                if *visited.get(&current_cell).unwrap_or(&false) {
                    continue;
                }

                self.heights[current_cell] = height.clone();

                if distribution.sample(&mut rng) < P_HEIGHT {
                    height -= rng.gen_range(0.01..0.1);
                }

                if (distribution.sample(&mut rng) < P_STOP) | (height < 0.1){
                    stop = true;
                }

                if !stop {
                    let neighbors = &self.cell_neighbors[current_cell];
                    for neighbor in neighbors {
                        queue.push_back(*neighbor);
                    }
                }
            }


        }
    }
        fn generate_plate_vectors(&mut self){
            let mut rng = rand::thread_rng();
            let mut vectors = Vec::new();
            for i in 0..self.seed_points.len() {
                let vector = Vec3::new(rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0));
                vectors.push(vector);
            }
            self.plate_vectors = vectors;
        }
        
}

