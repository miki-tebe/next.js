use std::collections::HashSet;

use anyhow::Result;
use auto_hash_map::AutoSet;
use turbo_tasks::{
    FxIndexMap, FxIndexSet, ResolvedVc, TryFlatJoinIterExt, TryJoinIterExt, Value, Vc,
};

use super::{
    availability_info::AvailabilityInfo, available_modules::AvailableModulesInfo, chunk_content,
    chunking::make_chunks, AsyncModuleInfo, Chunk, ChunkContentResult, ChunkItem, ChunkableModule,
    ChunkingContext,
};
use crate::{
    environment::ChunkLoading, module::Module, output::OutputAssets, rebase::RebasedAsset,
    reference::ModuleReference,
};

pub struct MakeChunkGroupResult {
    pub chunks: Vec<ResolvedVc<Box<dyn Chunk>>>,
    pub availability_info: AvailabilityInfo,
}

/// Creates a chunk group from a set of entries.
pub async fn make_chunk_group(
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    chunk_group_entries: impl IntoIterator<Item = ResolvedVc<Box<dyn Module>>>,
    availability_info: AvailabilityInfo,
) -> Result<MakeChunkGroupResult> {
    let can_split_async = matches!(
        *chunking_context.environment().chunk_loading().await?,
        ChunkLoading::Edge
    );

    let ChunkContentResult {
        chunkable_modules,
        external_module_references,
        async_modules,
        traced_modules,
        forward_edges_inherit_async,
        local_back_edges_inherit_async,
        available_async_modules_back_edges_inherit_async,
    } = chunk_content(
        chunk_group_entries,
        availability_info,
        can_split_async,
        *chunking_context.is_tracing_enabled().await?,
    )
    .await?;
    let external_output_assets = OutputAssets::empty();

    // Find all local chunk items that are self async
    let self_async_children = chunkable_modules
        .iter()
        .copied()
        .map(|m| async move {
            let is_self_async = *m.is_self_async().await?;
            Ok(is_self_async.then_some(m))
        })
        .try_flat_join()
        .await?;

    // Get all available async modules and concatenate with local async modules
    let mut async_chunk_items = available_async_modules_back_edges_inherit_async
        .keys()
        .copied()
        .chain(self_async_children.into_iter())
        .map(|m| (m, AutoSet::<Vc<Box<dyn ChunkableModule>>>::new()))
        .collect::<FxIndexMap<_, _>>();

    // Propagate async inheritance
    let mut i = 0;
    loop {
        let Some((&chunk_item, _)) = async_chunk_items.get_index(i) else {
            break;
        };
        // The first few entries are from
        // available_async_modules_back_edges_inherit_async and need to use that map,
        // all other entries are local
        let map = if i < available_async_modules_back_edges_inherit_async.len() {
            &available_async_modules_back_edges_inherit_async
        } else {
            &local_back_edges_inherit_async
        };
        if let Some(parents) = map.get(&chunk_item) {
            for &parent in parents.iter() {
                // Add item, it will be iterated by this loop too
                async_chunk_items
                    .entry(parent)
                    .or_default()
                    .insert(chunk_item);
            }
        }
        i += 1;
    }

    // Create map for chunk items with empty [Option<Vc<AsyncModuleInfo>>]
    let mut modules = chunkable_modules
        .into_iter()
        .map(|m| (m, None))
        .collect::<FxIndexMap<_, Option<Vc<AsyncModuleInfo>>>>();

    // Insert AsyncModuleInfo for every async module
    for (async_item, referenced_async_modules) in async_chunk_items {
        let referenced_async_modules =
            if let Some(references) = forward_edges_inherit_async.get(&async_item) {
                references
                    .iter()
                    .copied()
                    .filter(|item| referenced_async_modules.contains(item))
                    .collect()
            } else {
                Default::default()
            };
        modules.insert(
            async_item,
            Some(AsyncModuleInfo::new(referenced_async_modules)),
        );
    }

    // Compute new [AvailabilityInfo]
    let availability_info = {
        let map = modules
            .iter()
            .map(|(&module, async_info)| async move {
                Ok((
                    module.to_resolved().await?,
                    AvailableModulesInfo {
                        is_async: async_info.is_some(),
                    },
                ))
            })
            .try_join()
            .await?
            .into_iter()
            .collect();
        let map = Vc::cell(map);
        availability_info.with_modules(map).await?
    };

    // Insert async chunk loaders for every referenced async module
    let async_loaders = async_modules
        .into_iter()
        .map(|module| {
            chunking_context.async_loader_chunk_item(*module, Value::new(availability_info))
        })
        .collect::<Vec<_>>();
    let has_async_loaders = !async_loaders.is_empty();
    let async_loader_chunk_items = async_loaders.iter().map(|&chunk_item| (chunk_item, None));

    // And also add output assets referenced by async chunk loaders
    let async_loader_references = async_loaders
        .iter()
        .map(|&loader| loader.references())
        .try_join()
        .await?;
    let async_loader_external_module_references = Vc::cell(
        async_loader_references
            .iter()
            .flat_map(|references| references.iter().copied())
            .collect(),
    );

    let mut referenced_output_assets = (*external_output_assets.await?).clone();
    referenced_output_assets.extend(
        references_to_output_assets(external_module_references)
            .await?
            .await?
            .iter()
            .copied(),
    );

    let rebased_modules = traced_modules
        .into_iter()
        .map(|module| {
            RebasedAsset::new(
                *module,
                module.ident().path().root(),
                module.ident().path().root(),
            )
            .to_resolved()
        })
        .try_join()
        .await?;

    referenced_output_assets.extend(rebased_modules.into_iter().map(ResolvedVc::upcast));

    let chunk_items = modules
        .iter()
        .map(|(m, async_info)| (m.as_chunk_item(chunking_context), *async_info))
        .collect::<Vec<_>>();

    // Pass chunk items to chunking algorithm
    let mut chunks = make_chunks(
        chunking_context,
        Vc::cell(chunk_items),
        "".into(),
        Vc::cell(referenced_output_assets),
    )
    .await?
    .clone_value();

    if has_async_loaders {
        // Pass async chunk loaders to chunking algorithm
        // We want them to be separate since they are specific to this chunk group due
        // to available chunk items differing
        let async_loader_chunks = make_chunks(
            chunking_context,
            Vc::cell(async_loader_chunk_items.into_iter().collect()),
            "async-loader-".into(),
            async_loader_external_module_references,
        )
        .await?;

        // concatenate chunks
        chunks.extend(async_loader_chunks.iter().copied());
    }

    Ok(MakeChunkGroupResult {
        chunks,
        availability_info,
    })
}

pub async fn references_to_output_assets(
    references: FxIndexSet<Vc<Box<dyn ModuleReference>>>,
) -> Result<Vc<OutputAssets>> {
    let output_assets = references
        .into_iter()
        .map(|reference| reference.resolve_reference().primary_output_assets())
        .try_join()
        .await?;
    let mut set = HashSet::new();
    let output_assets = output_assets
        .iter()
        .flatten()
        .copied()
        .filter(|&asset| set.insert(asset))
        .map(|asset| *asset)
        .collect::<Vec<_>>();
    Ok(OutputAssets::new(output_assets))
}
