#![allow(unused_imports)]

use bevy::window::Window;
use bevy::{
    pbr::wireframe::{Wireframe, WireframeConfig, WireframePlugin},
    prelude::*,
    render::{
        mesh::{Indices, PrimitiveTopology},
        render_asset::RenderAssetUsages,
        settings::{RenderCreation, WgpuFeatures, WgpuSettings},
        RenderPlugin,
    },
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};
use bevy_mod_picking::prelude::*;
use bevy_pancam::{PanCam, PanCamPlugin};
use fast_poisson::Poisson2D;
use rand::Rng;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::{default, iter::zip};
use voronator::{
    delaunator::{Coord, Point},
    polygon::Polygon,
    CentroidDiagram, VoronoiDiagram,
};

#[derive(PartialEq, Debug, Clone, Copy)]
struct HashablePoint(Point);

impl Eq for HashablePoint {}

impl Hash for HashablePoint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.x.to_bits().hash(state);
        self.0.y.to_bits().hash(state);
    }
}

impl Coord for HashablePoint {
    fn x(&self) -> f64 {
        self.0.x
    }

    fn y(&self) -> f64 {
        self.0.y
    }

    fn from_xy(x: f64, y: f64) -> Self {
        HashablePoint(Point { x, y })
    }
}

impl From<Point> for HashablePoint {
    fn from(point: Point) -> Self {
        HashablePoint(point)
    }
}

#[derive(Debug, Clone, Copy, Component)]
struct PolygonId {
    point: HashablePoint,
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings {
                    // WARN this is a native only feature. It will not work with webgl or webgpu
                    features: WgpuFeatures::POLYGON_MODE_LINE,
                    ..default()
                }),
                ..default()
            }),
            WireframePlugin,
        ))
        .add_plugins((PanCamPlugin,))
        .add_plugins(DebugPickingPlugin)
        .insert_resource(WireframeConfig {
            global: true,
            default_color: Color::BLACK,
        })
        .add_systems(Startup, startup_system)
        .run();
}

fn startup_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    window: Query<&mut Window>,
) {
    let window = window.single();
    let width = (window.resolution.physical_width() / 2) as f64;
    let height = (window.resolution.physical_height() / 2) as f64;
    // const GRIDSIZE: f64 = 20.;

    let diagram = generate_centroid_diagram(
        Point { x: 0., y: 0. },
        Point {
            x: window.resolution.physical_width() as f64,
            y: window.resolution.physical_height() as f64,
        },
        15.,
    );

    let points = &diagram.cells;

    let mut polygon_hashmap: HashMap<HashablePoint, Polygon<HashablePoint>> = HashMap::new();

    for (polygon, center) in zip(diagram.cells.iter(), diagram.sites.iter()) {
        let hash_point = HashablePoint::from(*center);
        let points = polygon
            .points()
            .iter()
            .map(|point| HashablePoint::from(*point))
            .collect::<Vec<HashablePoint>>();
        polygon_hashmap.insert(hash_point, Polygon::from_points(points));
    }

    // for site in &diagram.sites{
    //     commands.spawn(MaterialMesh2dBundle{
    //         mesh: meshes.add(Mesh::from(Circle{radius: 3.0, ..Default::default()})).into(),
    //         material: materials.add(Color::RED),
    //         transform: Transform::from_translation(Vec3::new((site.x - width) as f32, (site.y - height) as f32, 10.)),
    //         ..Default::default()
    //     });
    // }

    let (poly_meshes, sites) = generate_centroid_mesh(&diagram);

    for (mesh, site) in zip(poly_meshes, sites) {
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: meshes.add(mesh).into(),
                material: materials.add(Color::WHITE),
                transform: Transform::from_translation(Vec3::new(
                    -width as f32,
                    -height as f32,
                    0.,
                )),
                ..Default::default()
            },
            PolygonId {
                point: HashablePoint::from(site),
            },
            PickableBundle::default(),
            On::<Pointer<Click>>::target_commands_mut(|_click, target_commands| {
                target_commands.despawn();
            }),
        ));
    }
    let outline_meshes = generate_polygon_outline_mesh(points, width, height);
    for mesh in outline_meshes {
        commands.spawn((MaterialMesh2dBundle {
            mesh: meshes.add(mesh).into(),
            material: materials.add(Color::BLACK),
            transform: Transform::from_translation(Vec3::new(-width as f32, -height as f32, 10.)),
            ..Default::default()
        },));
    }

    commands.spawn(Camera2dBundle::default()).insert(PanCam {
        grab_buttons: vec![MouseButton::Middle], // which buttons should drag the camera
        enabled: true,        // when false, controls are disabled. See toggle example.
        zoom_to_cursor: true, // whether to zoom towards the mouse or the center of the screen
        min_scale: 0.1,       // prevent the camera from zooming too far in
        max_scale: Some(2.),  // prevent the camera from zooming too far out
        ..default()
    });
}

fn generate_diagram(min: Point, max: Point, radius: f64) -> VoronoiDiagram<Point> {
    let points = generate_points_poisson(min, max, radius);
    let diagram = VoronoiDiagram::new(&min, &max, &points[..]).unwrap();
    diagram
}

fn generate_centroid_diagram(min: Point, max: Point, radius: f64) -> CentroidDiagram<Point> {
    let points = generate_points_poisson(min, max, radius);
    let diagram = CentroidDiagram::new(&points[..]).unwrap();
    diagram
}

fn generate_points_poisson(min: Point, max: Point, radius: f64) -> Vec<Point> {
    let sample = Poisson2D::new()
        .with_dimensions([max.x, max.y], radius)
        .generate();
    let points = sample
        .iter()
        .map(|p| Point { x: p[0], y: p[1] })
        .collect::<Vec<Point>>();
    points
}

fn generate_polygon_outline_mesh(
    polygons: &[Polygon<Point>],
    width: f64,
    height: f64,
) -> Vec<Mesh> {
    let mut meshes: Vec<Mesh> = Vec::new();
    for polygon in polygons.iter() {
        let mut points = Vec::new();
        for point in polygon.points() {
            points.push([(point.x) as f32, (point.y) as f32, 10.]);
        }
        let mesh = Mesh::new(PrimitiveTopology::LineStrip, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points);

        meshes.push(mesh);
    }
    meshes
}

fn generate_voronoi_mesh(diagram: &VoronoiDiagram<Point>) -> (Vec<Mesh>, Vec<Point>) {
    let mut meshes: Vec<Mesh> = Vec::new();

    for (polygon, center) in zip(diagram.cells().iter(), diagram.sites.iter()) {
        let mut points = Vec::new();
        points.push([center.x as f32, center.y as f32, 0.]);
        for point in polygon.points() {
            points.push([point.x as f32, point.y as f32, -1.]);
        }
        let mut indices = Vec::new();
        for i in 1..points.len() - 1 {
            indices.push(0);
            indices.push(i as u32);
            indices.push(i as u32 + 1);
        }
        indices.push(0);
        indices.push((points.len() - 1).try_into().unwrap());
        indices.push(1);

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, points.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; points.len()]);
        mesh.insert_indices(Indices::U32(indices));
        meshes.push(mesh);
    }
    (meshes, diagram.sites.clone())
}

fn generate_centroid_mesh(diagram: &CentroidDiagram<Point>) -> (Vec<Mesh>, Vec<Point>) {
    let mut meshes: Vec<Mesh> = Vec::new();

    for (polygon, center) in zip(diagram.cells.iter(), diagram.sites.iter()) {
        let mut points = Vec::new();
        points.push([center.x as f32, center.y as f32, 0.]);
        for point in polygon.points() {
            points.push([point.x as f32, point.y as f32, -1.]);
        }
        let mut indices = Vec::new();
        for i in 1..points.len() - 1 {
            indices.push(0);
            indices.push(i as u32);
            indices.push(i as u32 + 1);
        }
        indices.push(0);
        indices.push((points.len() - 1).try_into().unwrap());
        indices.push(1);

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, points.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; points.len()]);
        mesh.insert_indices(Indices::U32(indices));
        meshes.push(mesh);
    }
    (meshes, diagram.sites.clone())
}

pub struct CellMesh2d;
