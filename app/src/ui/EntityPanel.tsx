// Step 3.5: entity legend + selectable / editable list.
//
// Shows ONE list at a time — whichever entity layer is currently active
// (States / Provinces / Cultures / Religions). Only one entity layer can be
// displayed at once (per the entity-UI spec), so a single combined panel
// that follows the active layer is the natural legend.
//
// Each row renders:
//   - a colour swatch (the entity's `color`, 0xRRGGBB) — the legend key,
//   - the entity name (editable: click to rename),
//   - a colour picker (editable: change the fill colour),
//   - a "select" affordance: clicking the row selects the entity so the map
//     highlights every cell it owns (and, for states on the Provinces layer,
//     draws its border).
//
// Edits flow through the store's `updateEntity` (which mutates the matching
// entity in `statesResult` / `culturesResult` and re-uploads the entity
// colour texture via MapCanvas's entity-subscription).

import { useMemo } from "react";
import { rgb } from "../render/palette";
import { useWorldgenStore } from "../state/worldgenStore";
import type { EntityKind } from "../render/layers";
import type { WorldMap } from "../render/layers";

type EntityPanelProps = {
	/** Reserved for future map-coupled affordances. The panel drives the map
	 * through the store (selectedEntity), so no direct WorldMap handle is
	 * required today. */
	worldMap?: WorldMap | null;
};

/** 0xRRGGBB -> "#rrggbb" CSS string. */
function hexCss(color: number): string {
	const [r, g, b] = rgb(color);
	return `#${r.toString(16).padStart(2, "0")}${g
		.toString(16)
		.padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

/** "#rrggbb" -> 0xRRGGBB number. Returns null on malformed input. */
function cssToColor(css: string): number | null {
	const m = /^#?([0-9a-f]{6})$/i.exec(css.trim());
	if (!m) return null;
	return Number.parseInt(m[1], 16);
}

const LAYER_LABEL: Record<EntityKind, string> = {
	state: "States",
	province: "Provinces",
	culture: "Cultures",
	religion: "Religions",
};

export function EntityPanel({ worldMap: _worldMap }: EntityPanelProps): React.ReactElement | null {
	const grid = useWorldgenStore((s) => s.grid);
	const layerEnabled = useWorldgenStore((s) => s.layerEnabled);
	const statesResult = useWorldgenStore((s) => s.statesResult);
	const culturesResult = useWorldgenStore((s) => s.culturesResult);
	const selectedEntity = useWorldgenStore((s) => s.selectedEntity);
	const selectEntity = useWorldgenStore((s) => s.selectEntity);
	const updateEntity = useWorldgenStore((s) => s.updateEntity);

	// Which entity kind is the single active layer?
	const activeKind: EntityKind | null = useMemo(() => {
		if (layerEnabled.states) return "state";
		if (layerEnabled.provinces) return "province";
		if (layerEnabled.cultures) return "culture";
		if (layerEnabled.religions) return "religion";
		return null;
	}, [layerEnabled]);

	// Build the list of {id, name, color} for the active kind.
	const items = useMemo(() => {
		if (!activeKind) return [];
		if (activeKind === "state") {
			if (!statesResult) return [];
			return statesResult.pack.states.map((e) => ({
				id: e.id,
				name: e.name,
				color: e.color,
			}));
		}
		if (activeKind === "province") {
			if (!statesResult) return [];
			return statesResult.pack.provinces.map((e) => ({
				id: e.id,
				name: e.name,
				color: e.color,
			}));
		}
		if (activeKind === "culture") {
			if (!culturesResult) return [];
			return culturesResult.cultures.map((e) => ({
				id: e.id,
				name: e.name,
				color: e.color,
			}));
		}
		// religion
		if (!culturesResult) return [];
		return culturesResult.religions.map((e) => ({
			id: e.id,
			name: e.name,
			color: e.color,
		}));
	}, [activeKind, statesResult, culturesResult]);

	if (!grid) return null;

	if (!activeKind) {
		return (
			<div
				data-testid="entity-panel"
				style={{
					padding: "0.5rem",
					borderTop: "1px solid #30363d",
					fontSize: "0.8rem",
					color: "#6e7681",
				}}
			>
				Enable a States / Provinces / Cultures / Religions layer to
				view its legend.
			</div>
		);
	}

	return (
		<div
			data-testid="entity-panel"
			data-entity-kind={activeKind}
			style={{
				display: "flex",
				flexDirection: "column",
				gap: "0.35rem",
				padding: "0.5rem",
				borderTop: "1px solid #30363d",
				fontSize: "0.8rem",
				color: "#e6edf3",
			}}
		>
			<span
				style={{
					fontSize: "0.72rem",
					color: "#8b949e",
					textTransform: "uppercase",
					letterSpacing: "0.05em",
				}}
			>
				{LAYER_LABEL[activeKind]} legend
			</span>

			<div
				style={{
					display: "flex",
					flexDirection: "column",
					gap: "0.2rem",
					maxHeight: "40vh",
					overflowY: "auto",
				}}
			>
				{items.map((item) => {
					const isSelected =
						!!selectedEntity &&
						selectedEntity.kind === activeKind &&
						selectedEntity.id === item.id;
					return (
						<div
							key={item.id}
							data-testid={`entity-row-${activeKind}-${item.id}`}
							onClick={() => {
								const sel = isSelected ? null : { kind: activeKind, id: item.id };
								selectEntity(sel);
							}}
							style={{
								display: "flex",
								alignItems: "center",
								gap: "0.4rem",
								padding: "0.25rem 0.35rem",
								borderRadius: "4px",
								cursor: "pointer",
								background: isSelected ? "#21262d" : "transparent",
								border: isSelected
									? "1px solid #2f81f7"
									: "1px solid transparent",
							}}
						>
							{/* Colour swatch = the legend key. */}
							<span
								title="Entity colour"
								style={{
									width: "14px",
									height: "14px",
									borderRadius: "3px",
									flex: "0 0 auto",
									background: hexCss(item.color),
									border: "1px solid #30363d",
								}}
							/>
							{/* Editable name. */}
							<input
								value={item.name}
								onClick={(e) => e.stopPropagation()}
								onChange={(e) =>
									updateEntity(activeKind, item.id, {
										name: e.target.value,
									})
								}
								style={{
									flex: "1 1 auto",
									minWidth: 0,
									background: "#0d1117",
									color: "#e6edf3",
									border: "1px solid #30363d",
									borderRadius: "3px",
									fontSize: "0.78rem",
									padding: "0.15rem 0.3rem",
								}}
								data-testid={`entity-name-${activeKind}-${item.id}`}
							/>
							{/* Editable colour. */}
							<input
								type="color"
								value={hexCss(item.color)}
								onClick={(e) => e.stopPropagation()}
								onChange={(e) => {
									const c = cssToColor(e.target.value);
									if (c !== null)
										updateEntity(activeKind, item.id, { color: c });
								}}
								style={{
									width: "22px",
									height: "22px",
									flex: "0 0 auto",
									padding: 0,
									border: "none",
									background: "transparent",
									cursor: "pointer",
								}}
								data-testid={`entity-color-${activeKind}-${item.id}`}
								title="Change colour"
							/>
						</div>
					);
				})}
				{items.length === 0 && (
					<span style={{ color: "#6e7681", fontSize: "0.78rem" }}>
						No entities generated yet.
					</span>
				)}
			</div>
		</div>
	);
}

export default EntityPanel;
