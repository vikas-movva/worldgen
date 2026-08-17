// Step 3.5: entity inspector (attributes + History tab).
//
// Sits beside the EntityPanel in the sidebar. Reads the currently selected
// entity from the zustand store (set by the panel list or by click-to-select on
// the map) and renders:
//   - an Attributes tab — the entity's base fields (id, name, color, capital,
//     population, founded/dissolved year, ...) as a labelled readout, and
//   - a History tab — renders timeline events (Phase 5.1) filtered to the
//     selected entity. If no timeline exists yet or no events match this
//     entity, shows a placeholder + the year-0 anchor facts. Once
//     `generateTimeline` runs, the chronicle fills in the entity's
//     succession / schism / conquest events. The component is structured so
//     the History tab can be swapped for a TipTap editor (Step 6.1) with no
//     prop changes.
//
// The inspector renders nothing when no entity is selected — it is purely a
// readout, never an editor (entity name/color edits live in EntityPanel).

import { useMemo } from "react";
import type {
	TimelineEvent,
} from "../core/api";
import type {
	Culture,
	Religion,
	State,
} from "../state/types";
import { useWorldgenStore } from "../state/worldgenStore";
import { useTimelineScrub } from "./useTimelineScrub";

type EntityKind = "state" | "province" | "culture" | "religion";

/** 0xRRGGBB -> "#rrggbb" CSS string. */
function hexCss(color: number): string {
  const r = (color >> 16) & 0xff;
  const g = (color >> 8) & 0xff;
  const b = color & 0xff;
  return `#${r.toString(16).padStart(2, "0")}${g
    .toString(16)
    .padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

function fmtYear(y: number | null | undefined): string {
  if (y === null || y === undefined) return "extant";
  return String(y);
}

function fmtPop(n: number | undefined): string {
  if (n === undefined || n === null) return "?";
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

type Attr = { label: string; value: string };

function buildAttrs(
  kind: EntityKind,
  entity: State | { id: number; name: string; color: number } | Culture | Religion,
): Attr[] {
  const rows: Attr[] = [
    { label: "Kind", value: kind[0].toUpperCase() + kind.slice(1) },
    { label: "ID", value: String(entity.id) },
    { label: "Name", value: entity.name },
    { label: "Color", value: hexCss(entity.color) },
  ];
  const s = entity as State;
  if (kind === "state") {
    rows.push(
      { label: "Capital", value: s.capital ? String(s.capital) : "none" },
      { label: "Center cell", value: String(s.center_cell) },
      { label: "Form", value: s.form || "?" },
      { label: "Tax rate", value: s.tax_rate != null ? String(s.tax_rate) : "?" },
      { label: "Treasury", value: fmtPop(s.treasury) },
      { label: "Rural pop", value: fmtPop(s.rural_pop) },
      { label: "Urban pop", value: fmtPop(s.urban_pop) },
      { label: "Military", value: String(s.military ?? "?") },
      { label: "Culture", value: String(s.culture ?? "?") },
    );
  }
  const p = entity as { state?: number; center_cell?: number; rural_pop?: number; urban_pop?: number };
  if (kind === "province") {
    rows.push(
      { label: "State", value: String(p.state ?? "?") },
      { label: "Center cell", value: String(p.center_cell ?? "?") },
      { label: "Rural pop", value: fmtPop(p.rural_pop) },
      { label: "Urban pop", value: fmtPop(p.urban_pop) },
    );
  }
  const c = entity as Culture;
  if (kind === "culture") {
    rows.push(
      { label: "Origin cell", value: String(c.origin) },
      { label: "Type", value: String(c.type_code) },
      { label: "Cells", value: String(c.cell_count) },
    );
  }
  const r = entity as Religion;
  if (kind === "religion") {
    rows.push(
      { label: "Center cell", value: String(r.center_cell) },
      { label: "Parent", value: r.parent != null ? String(r.parent) : "root" },
      { label: "Followers", value: fmtPop(r.followers) },
      { label: "Type", value: String(r.type_code) },
    );
  }
  // founded / dissolved are common to all entity types.
  const f = (entity as { founded_year?: number; dissolved_year?: number | null })
    .founded_year;
  const d = (entity as { dissolved_year?: number | null }).dissolved_year;
  rows.push(
    { label: "Founded", value: fmtYear(f) },
    { label: "Dissolved", value: fmtYear(d) },
  );
  return rows;
}

export function EntityInspector(): React.ReactElement | null {
  const selectedEntity = useWorldgenStore((s) => s.selectedEntity);
  const statesResult = useWorldgenStore((s) => s.statesResult);
  const culturesResult = useWorldgenStore((s) => s.culturesResult);
  const timeline = useWorldgenStore((s) => s.timeline);
  const scrubTo = useTimelineScrub();

  const { kind, entity, attrs, events } = useMemo(() => {
    if (!selectedEntity) return { kind: null, entity: null, attrs: [], events: [] };
    const k = selectedEntity.kind as EntityKind;
    const i = selectedEntity.id;
    let ent: State | Culture | Religion | { id: number; name: string; color: number } | null = null;
    if (k === "state" || k === "province") {
      const vec = k === "state"
        ? statesResult?.pack.states
        : statesResult?.pack.provinces;
      ent = vec?.find((e) => e.id === i) ?? null;
    } else {
      const vec = k === "culture"
        ? culturesResult?.cultures
        : culturesResult?.religions;
      ent = vec?.find((e) => e.id === i) ?? null;
    }
    // Phase 5.1: filter timeline events matching this entity. Map the
    // EntityKind to the capitalized EntityType used by TimelineEvent.
    const entityTypeMap: Record<EntityKind, string> = {
      state: "State",
      province: "Province",
      culture: "Culture",
      religion: "Religion",
    };
    const targetType = entityTypeMap[k];
    const filtered = timeline
      ? timeline.filter(
          (ev: TimelineEvent) =>
            ev.entity_type === targetType && ev.entity_id === i,
        )
      : [];
    return { kind: k, entity: ent, attrs: ent ? buildAttrs(k, ent) : [], events: filtered };
  }, [selectedEntity, statesResult, culturesResult, timeline]);

  if (!kind || !entity) {
    return (
      <div
        data-testid="entity-inspector"
        style={{
          padding: "0.5rem",
          borderTop: "1px solid #21262d",
          fontSize: "0.8rem",
          color: "#6e7681",
        }}
      >
        Select an entity in the legend to inspect it.
      </div>
    );
  }

  return (
    <div
      data-testid="entity-inspector"
      data-entity-kind={kind}
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.45rem",
        padding: "0.5rem",
        borderTop: "1px solid #30363d",
        fontSize: "0.82rem",
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
        {kind} inspector
      </span>

      {/* Header: color swatch + name. */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "0.4rem",
        }}
      >
        <span
          title="Entity colour"
          style={{
            width: "16px",
            height: "16px",
            borderRadius: "3px",
            flex: "0 0 auto",
            background: hexCss(entity.color),
            border: "1px solid #30363d",
          }}
        />
        <span style={{ fontWeight: 600, wordBreak: "break-word" }}>
          {entity.name}
        </span>
      </div>

      {/* Attributes readout. */}
      <table
        style={{
          width: "100%",
          borderCollapse: "collapse",
          fontSize: "0.78rem",
          fontFamily: "monospace",
        }}
      >
        <tbody>
          {attrs.map((row) => (
            <tr key={row.label}>
              <td
                style={{
                  color: "#8b949e",
                  paddingRight: "0.5rem",
                  verticalAlign: "top",
                }}
              >
                {row.label}
              </td>
              <td style={{ color: "#e6edf3", fontWeight: 500 }}>
                {row.value}
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {/* History tab — populated from timeline events (Phase 5.1). */}
      <div
        style={{
          borderTop: "1px solid #21262d",
          paddingTop: "0.4rem",
        }}
      >
        <span
          style={{
            fontSize: "0.72rem",
            color: "#8b949e",
            textTransform: "uppercase",
            letterSpacing: "0.05em",
            display: "block",
            marginBottom: "0.3rem",
          }}
        >
          History
        </span>
        {events.length > 0 ? (
          <div
            style={{
              fontSize: "0.78rem",
              color: "#e6edf3",
            }}
          >
            <table
              style={{
                width: "100%",
                borderCollapse: "collapse",
                fontSize: "0.76rem",
                fontFamily: "monospace",
              }}
            >
              <tbody>
                {events.map((ev: TimelineEvent) => (
                  <tr key={ev.id}>
                    <td
                      style={{
                        color: "#8b949e",
                        paddingRight: "0.4rem",
                        verticalAlign: "top",
                        cursor: "pointer",
                        textDecoration: "underline",
                        textUnderlineOffset: "1px",
                      }}
                      onClick={() => scrubTo(ev.year)}
                      title={`Jump to year ${ev.year}`}
                    >
                      {ev.year}
                    </td>
                    <td
                      style={{
                        color: "#6e7681",
                        paddingRight: "0.4rem",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {ev.kind}
                    </td>
                    <td style={{ color: "#e6edf3" }}>
                      {ev.narrative ?? <span style={{ fontStyle: "italic", opacity: 0.6 }}>No narrative.</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div
            style={{
              fontSize: "0.78rem",
              color: "#6e7681",
              fontStyle: "italic",
              lineHeight: 1.5,
            }}
          >
            No chronicle events for this entity yet. The timeline engine
            (Phase 5.1) populates this tab with the entity's succession /
            schism / conquest events once <code>generateTimeline</code>
            runs. Today the year-0 anchor facts above are the only history
            available.
          </div>
        )}
        <div
          style={{
            marginTop: "0.3rem",
            fontSize: "0.72rem",
            color: "#8b949e",
            fontFamily: "monospace",
          }}
        >
          Year-0 anchor: founded{" "}
          {fmtYear((entity as { founded_year?: number }).founded_year)}
          {", dissolved "}
          {fmtYear(
            (entity as { dissolved_year?: number | null }).dissolved_year,
          )}
        </div>
      </div>
    </div>
  );
}

export default EntityInspector;