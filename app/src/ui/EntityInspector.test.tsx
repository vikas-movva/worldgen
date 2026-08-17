// Step 3.5 unit tests — EntityInspector (attributes readout).
//
// These exercise the inspector's render WITHOUT a real Web Worker or WASM:
//   - the "select an entity" hint when no entity is selected.
//   - the Attributes readout for each of the four entity kinds (state /
//     province / culture / religion) — every row the component promises is
//     present and carries the right value.
//   - the History tab shows the year-0 anchor line (founded / dissolved).
//   - the component is a pure readout: selecting an entity that doesn't exist
//     in the current pack renders the hint, not a crash.
//
// The inspector is rendered into a jsdom container via `react-dom/client`
// (same harness as `CellInspector.test.tsx`). No fake worker is needed —
// the inspector never calls `coreApi`.

import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Religion } from "../state/types";
import { useWorldgenStore } from "../state/worldgenStore";
import { EntityInspector } from "./EntityInspector";

function renderInspector(): { container: HTMLElement; unmount: () => void } {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(<EntityInspector />);
  });
  return {
    container,
    unmount: () => {
      act(() => {
        root.unmount();
      });
      container.remove();
    },
  };
}

beforeEach(() => {
  useWorldgenStore.setState({
    grid: null,
    mesh: null,
    climate: null,
    generation: null,
    layerEnabled: {
      terrain: true,
      biome: false,
      rivers: false,
      lakes: false,
      states: false,
      provinces: false,
      cultures: false,
      religions: false,
    },
    selectedEntity: null,
    statesResult: null,
    culturesResult: null,
  });
});

afterEach(() => {
  useWorldgenStore.setState({ selectedEntity: null });
});

// ---- fixtures -------------------------------------------------------------

function fakeStatesResult() {
  return {
    pack: {
      states: [
        {
          id: 1,
          name: "The Crown",
          color: 0x1f6feb,
          capital: 1,
          center_cell: 42,
          form: "Monarchy",
          tax_rate: 0.15,
          treasury: 1200.5,
          rural_pop: 45000,
          urban_pop: 8000,
          military: 120,
          founded_year: -200,
          dissolved_year: null,
          culture: 1,
        },
      ],
      provinces: [
        {
          id: 1,
          state: 1,
          name: "Western March",
          color: 0x2ea043,
          center_cell: 100,
          rural_pop: 20000,
          urban_pop: 1200,
          founded_year: -180,
          dissolved_year: null,
        },
      ],
      cultures: [],
      religions: [],
      burgs: [],
      armies: [],
    },
    cells_state: [],
    cells_province: [],
    cells_burg: [],
  };
}

function fakeCulturesResult() {
  return {
    cultures: [
      {
        id: 0,
        name: "Riverfolk",
        color: 0xd29922,
        origin: 7,
        type_code: 2,
        founded_year: -210,
        dissolved_year: null,
        cell_count: 340,
      },
    ],
    religions: [
      {
        id: 0,
        name: "The Ember Faith",
        color: 0xf85149,
        center_cell: 42,
        parent: null,
        followers: 9200,
        type_code: 0,
        expansion_mode: "global",
        founded_year: -190,
        dissolved_year: null,
      },
    ],
    cells_culture: [],
    cells_religion: [],
  };
}

// ---- tests ----------------------------------------------------------------

describe("EntityInspector — no selection", () => {
  it("renders the 'select an entity' hint when nothing is selected", () => {
    const { container, unmount } = renderInspector();
    const el = container.querySelector('[data-testid="entity-inspector"]');
    expect(el?.textContent).toMatch(/select an entity/i);
    unmount();
  });

  it("renders the hint when the selected entity id is not in the pack", () => {
    act(() =>
      useWorldgenStore.setState({
        selectedEntity: { kind: "state", id: 999 },
        statesResult: fakeStatesResult(),
      }),
    );
    const { container, unmount } = renderInspector();
    expect(container.textContent).toMatch(/select an entity/i);
    unmount();
  });
});

describe("EntityInspector — state readout", () => {
  beforeEach(() => {
    useWorldgenStore.setState({
      statesResult: fakeStatesResult(),
      culturesResult: fakeCulturesResult(),
      selectedEntity: { kind: "state", id: 1 },
    });
  });

  it("renders the kind / id / name / color header", () => {
    const { container, unmount } = renderInspector();
    const text = container.textContent ?? "";
    expect(text).toMatch(/State/);
    expect(text).toMatch(/1/);
    expect(text).toMatch(/The Crown/);
    unmount();
  });

  it("renders every state attribute row with the right value", () => {
    const { container, unmount } = renderInspector();
    const text = container.textContent ?? "";
    expect(text).toMatch(/Capital/);
    expect(text).toMatch(/1/);
    expect(text).toMatch(/Center cell/);
    expect(text).toMatch(/42/);
    expect(text).toMatch(/Monarchy/);
    expect(text).toMatch(/0\.15/);
    expect(text).toMatch(/1\.2k/); // treasury 1200.5 -> 1.2k
    expect(text).toMatch(/45\.0k/); // rural_pop 45000
    expect(text).toMatch(/8\.0k/); // urban_pop 8000
    expect(text).toMatch(/120/);
    expect(text).toMatch(/Culture/);
    expect(text).toMatch(/1/);
    unmount();
  });

  it("shows the founded / dissolved year in the History tab", () => {
    const { container, unmount } = renderInspector();
    const text = container.textContent ?? "";
    expect(text).toMatch(/History/);
    expect(text).toMatch(/founded -200/);
    expect(text).toMatch(/dissolved extant/);
    unmount();
  });
});

describe("EntityInspector — province readout", () => {
  beforeEach(() => {
    useWorldgenStore.setState({
      statesResult: fakeStatesResult(),
      culturesResult: fakeCulturesResult(),
      selectedEntity: { kind: "province", id: 1 },
    });
  });

  it("renders province-specific rows (state / center cell / pops)", () => {
    const { container, unmount } = renderInspector();
    const text = container.textContent ?? "";
    expect(text).toMatch(/Province/);
    expect(text).toMatch(/Western March/);
    expect(text).toMatch(/State/);
    expect(text).toMatch(/1/);
    expect(text).toMatch(/Center cell/);
    expect(text).toMatch(/100/);
    expect(text).toMatch(/20\.0k/); // rural_pop 20000
    expect(text).toMatch(/1\.2k/); // urban_pop 1200
    unmount();
  });
});

describe("EntityInspector — culture readout", () => {
  beforeEach(() => {
    useWorldgenStore.setState({
      statesResult: fakeStatesResult(),
      culturesResult: fakeCulturesResult(),
      selectedEntity: { kind: "culture", id: 0 },
    });
  });

  it("renders culture-specific rows (origin / type / cells)", () => {
    const { container, unmount } = renderInspector();
    const text = container.textContent ?? "";
    expect(text).toMatch(/Culture/);
    expect(text).toMatch(/Riverfolk/);
    expect(text).toMatch(/Origin cell/);
    expect(text).toMatch(/7/);
    expect(text).toMatch(/Type/);
    expect(text).toMatch(/2/);
    expect(text).toMatch(/Cells/);
    expect(text).toMatch(/340/);
    unmount();
  });
});

describe("EntityInspector — religion readout", () => {
  beforeEach(() => {
    useWorldgenStore.setState({
      statesResult: fakeStatesResult(),
      culturesResult: fakeCulturesResult(),
      selectedEntity: { kind: "religion", id: 0 },
    });
  });

  it("renders religion-specific rows (center / parent / followers / type)", () => {
    const { container, unmount } = renderInspector();
    const text = container.textContent ?? "";
    expect(text).toMatch(/Religion/);
    expect(text).toMatch(/The Ember Faith/);
    expect(text).toMatch(/Center cell/);
    expect(text).toMatch(/42/);
    expect(text).toMatch(/Parent/);
    expect(text).toMatch(/root/);
    expect(text).toMatch(/Followers/);
    expect(text).toMatch(/9\.2k/); // followers 9200
    expect(text).toMatch(/Type/);
    expect(text).toMatch(/0/);
    unmount();
  });

  it("shows the schism parent when present (a dissolved child religion)", () => {
    const cr = fakeCulturesResult();
    (cr.religions[0] as Religion).parent = 5; // TS: parent is number|null, test fixture overrides
    cr.religions[0].dissolved_year = null;
    useWorldgenStore.setState({
      statesResult: fakeStatesResult(),
      culturesResult: cr,
      selectedEntity: { kind: "religion", id: 0 },
    });
    const { container, unmount } = renderInspector();
    expect(container.textContent).toMatch(/5/);
    unmount();
  });
});