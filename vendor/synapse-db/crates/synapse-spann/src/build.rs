//! SPANN build: k-means cluster assignment → write centroids + posting lists.

use anyhow::Result;
use linfa::DatasetBase;
use linfa::traits::Fit;
use linfa_clustering::KMeans;
use ndarray::Array2;
use std::{fs, path::Path};

use crate::{DocumentEmbedding, posting::write_posting};

/// Assign each doc to nearest centroid (L2).
pub fn assign_centroid(centroids: &Array2<f32>, vec: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::MAX;
    let _dim = vec.len();
    for (c_idx, row) in centroids.rows().into_iter().enumerate() {
        let d: f32 = row
            .iter()
            .zip(vec.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum();
        if d < best_d {
            best_d = d;
            best = c_idx;
        }
    }
    best
}

/// Run k-means on all doc vectors, write:
///   <dir>/centroids.bin
///   <dir>/posting/<id>.bin  (one per cluster)
pub fn build_index(
    dir: &Path,
    docs: &[DocumentEmbedding],
    n_clusters: usize,
    dim: usize,
    max_iter: u64,
) -> Result<Vec<Vec<f32>>> {
    let n = docs.len();
    // Build ndarray for linfa
    let flat: Vec<f64> = docs
        .iter()
        .flat_map(|(_, v)| v.iter().map(|x| *x as f64))
        .collect();
    let arr = Array2::from_shape_vec((n, dim), flat)?;
    let dataset = DatasetBase::from(arr);

    let k = n_clusters.min(n);
    let model = KMeans::params(k).max_n_iterations(max_iter).fit(&dataset)?;

    // Centroids as f32
    let centroids_f32: Vec<Vec<f32>> = model
        .centroids()
        .rows()
        .into_iter()
        .map(|r| r.iter().map(|x| *x as f32).collect())
        .collect();

    // Centroid f32 Array2 for assignment
    let flat_c: Vec<f32> = centroids_f32.iter().flatten().copied().collect();
    let centroids_arr = Array2::from_shape_vec((k, dim), flat_c.clone())?;

    // Assign each doc
    let mut clusters: Vec<Vec<DocumentEmbedding>> = vec![vec![]; k];
    for (docid, vec) in docs {
        let c = assign_centroid(&centroids_arr, vec);
        clusters[c].push((*docid, vec.clone()));
    }

    // Write centroids.bin
    let cbin: Vec<u8> = flat_c.iter().flat_map(|x| x.to_le_bytes()).collect();
    fs::write(dir.join("centroids.bin"), cbin)?;

    // Write posting lists
    let posting_dir = dir.join("posting");
    fs::create_dir_all(&posting_dir)?;
    for (c_idx, entries) in clusters.iter().enumerate() {
        let p = posting_dir.join(format!("{c_idx}.bin"));
        write_posting(&p, entries, dim)?;
    }

    Ok(centroids_f32)
}

/// Load centroids.bin → Vec<Vec<f32>>
pub fn load_centroids(path: &Path, n_clusters: usize, dim: usize) -> Result<Vec<Vec<f32>>> {
    let bytes = fs::read(path)?;
    let expected = n_clusters * dim * 4;
    anyhow::ensure!(bytes.len() == expected, "centroids.bin size mismatch");
    let floats: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    Ok(floats.chunks(dim).map(|c| c.to_vec()).collect())
}
