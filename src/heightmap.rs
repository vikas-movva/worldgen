use std::collections::{HashMap, VecQueue};
use voronator::{delaunator::Triangulation, polygon::Polygon, CentroidDiagram, VoronoiDiagram};
use bevy::prelude::Vec3;
use maybe_parallel_iterator::IntoMaybeParallelIterator;
pub struct Heightmap{
    pub triangulation: Triangulation,
    pub heights: f64,
    pub cells: Vec<Polygon<HashablePoint>>,
    pub cell_neighbors: Vec<Vec<usize>>,
    pub seed_points: Vec<HashablePoint>,
    pub plate_vectors: Vec3<f64>,
}

impl Heightmap{
    pub fn new(diagram: D) -> Self
    where D: CentroidDiagram + VoronoiDiagram
    {
        
        Self{
            triangulation: diagram.delaunay,
            heights: vec![0.0; diagram.cells.len()],
            cells: diagram.cells,
            cell_neighbors: diagram.neighbors,
        }
    }
    pub fn generate_heights(&mut self, seed_points: Vec<HashablePoint>, plate_vectors: Vec3<f64>){
        self.seed_points = seed_points;
        self.plate_vectors = plate_vectors;
        let visited:HashMap<HashablePoint, bool> = HashMap::new();
        for seed in seed_points.into_maybe_par_iter(){
            let mut queue = VecQueue::new();
        }
        
    }
}