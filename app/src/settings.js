import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const EMPTY_PROFILE = {
  name: "", imo: "", callSign: "", mmsi: "", draftM: null, lengthM: null,
  breadthM: null, airDraftM: null, referenceLat: null, referenceLon: null,
  gpsAntennaFromBowM: null, gpsAntennaFromCenterlineM: null,
};

const INPUTS = Object.keys(EMPTY_PROFILE);

function optionalNumber(value) {
  if (value == null || value === "") return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function profileFromLegacy(boat) {
  return {
    ...EMPTY_PROFILE,
    mmsi: boat.mmsi || "",
    airDraftM: boat.airDraft > 0 ? boat.airDraft : null,
    lengthM: boat.length > 0 ? boat.length : null,
    breadthM: boat.beam > 0 ? boat.beam : null,
    gpsAntennaFromBowM: boat.gpsFromBow > 0 ? boat.gpsFromBow : null,
    gpsAntennaFromCenterlineM: boat.gpsFromCentre || null,
  };
}

function applyToBoat(profile) {
  window.dispatchEvent(new CustomEvent("racenav-vessel-profile", { detail: {
    name: profile.name,
    imo: profile.imo,
    callSign: profile.callSign,
    mmsi: profile.mmsi,
    draft: profile.draftM ?? 0,
    length: profile.lengthM ?? 0,
    beam: profile.breadthM ?? 0,
    airDraft: profile.airDraftM ?? 0,
    referenceLat: profile.referenceLat,
    referenceLon: profile.referenceLon,
    gpsFromBow: profile.gpsAntennaFromBowM ?? 0,
    gpsFromCentre: profile.gpsAntennaFromCenterlineM ?? 0,
  } }));
}

export function setupSettings({ boat, ui, onReferencePosition }) {
  const form = document.getElementById("vessel-settings");
  const status = document.getElementById("settings-status");
  const saveButton = form.querySelector('button[type="submit"]');
  const sourceForm = document.getElementById("nmea-source-form");
  const sourceName = sourceForm.elements.namedItem("name");
  const sourceType = sourceForm.elements.namedItem("type");
  const portSelect = sourceForm.elements.namedItem("port");
  const sourceList = document.getElementById("nmea-sources");
  const sourceStatus = document.getElementById("nmea-settings-status");
  const nmeaCurrentData = document.getElementById("nmea-current-data");
  const nmeaActiveSources = document.getElementById("nmea-active-sources");
  const nmeaRawViewer = document.getElementById("nmea-raw-viewer");
  const arbitrationBox = document.getElementById("nmea-arbitration");
  const failoverCheckbox = document.getElementById("nmea-failover-enabled");
  const staleAfterSelect = document.getElementById("nmea-stale-seconds");
  let saved = null;
  let sources = [];
  let arbitration = { failoverEnabled: true, staleAfterSeconds: 10, preferredSources: {} };
  let sourceStatuses = new Map();
  let activeGroupSources = {};
  const latestObservations = new Map();
  const recentSentences = [];
  let editingId = null;

  const showStatus = (text, kind = "") => {
    status.textContent = text;
    status.className = kind;
  };
  const showSourceStatus = (text, kind = "") => {
    sourceStatus.textContent = text;
    sourceStatus.className = kind;
  };
  function fill(profile) {
    const values = { ...EMPTY_PROFILE, ...profile };
    for (const key of INPUTS) {
      const input = form.elements.namedItem(key);
      input.value = values[key] ?? "";
    }
  }
  function applyReferencePosition(profile) {
    if (Number.isFinite(profile.referenceLat) && Number.isFinite(profile.referenceLon)) {
      onReferencePosition?.([profile.referenceLon, profile.referenceLat]);
    }
  }
  ui.onChange((pane) => {
    if (pane === "settings") form.querySelector("input")?.focus();
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!saved || !form.reportValidity()) return;
    const profile = {};
    for (const key of INPUTS) {
      const input = form.elements.namedItem(key);
      profile[key] = input.type === "number" ? optionalNumber(input.value) : input.value.trim();
    }
    const next = { ...saved, vessel: { ...saved.vessel, ...profile } };
    saveButton.disabled = true;
    showStatus("Saving…");
    try {
      await invoke("settings_save", { settings: next });
      saved = next;
      applyToBoat(profile);
      applyReferencePosition(profile);
      showStatus("Vessel details saved.", "good");
    } catch (error) {
      showStatus(`Could not save vessel details: ${error}`, "bad");
    } finally {
      saveButton.disabled = false;
    }
  });

  function syncTransportFields() {
    for (const row of sourceForm.querySelectorAll("[data-nmea-transport]")) {
      const active = row.dataset.nmeaTransport === sourceType.value;
      row.hidden = !active;
      for (const input of row.querySelectorAll("input, select")) input.required = active && input.name !== "baudRate";
    }
  }
  sourceType.addEventListener("change", syncTransportFields);
  syncTransportFields();

  async function scanPorts(selectedOverride = portSelect.value) {
    const selected = selectedOverride;
    try {
      const ports = await invoke("nmea_ports");
      portSelect.replaceChildren(new Option("Select a COM port", ""));
      for (const port of ports) {
        portSelect.add(new Option(`${port.name} · ${port.kind}`, port.name));
      }
      if (selected && !ports.some((port) => port.name === selected)) {
        portSelect.add(new Option(`${selected} · saved (not currently detected)`, selected));
      }
      portSelect.value = selected;
      showSourceStatus(ports.length ? `${ports.length} serial port${ports.length === 1 ? "" : "s"} detected.` : "No serial ports detected.", ports.length ? "good" : "");
    } catch (error) {
      showSourceStatus(`Could not scan serial ports: ${error}`, "bad");
    }
  }
  document.getElementById("nmea-scan-ports").addEventListener("click", scanPorts);

  function resetSourceForm() {
    editingId = null;
    sourceForm.reset();
    sourceType.value = "serial";
    sourceForm.elements.namedItem("baudRate").value = "4800";
    document.getElementById("nmea-form-title").textContent = "Add source";
    document.getElementById("nmea-source-save").textContent = "Add source";
    document.getElementById("nmea-source-cancel").hidden = true;
    syncTransportFields();
    portSelect.value = "";
  }

  function editSource(source) {
    editingId = source.id;
    sourceName.value = source.name;
    sourceType.value = source.transport.type;
    if (source.transport.type === "serial") {
      sourceForm.elements.namedItem("baudRate").value = String(source.transport.baudRate);
    } else if (source.transport.type === "tcp") {
      sourceForm.elements.namedItem("address").value = source.transport.address;
    } else {
      sourceForm.elements.namedItem("bindAddress").value = source.transport.bindAddress;
    }
    document.getElementById("nmea-form-title").textContent = "Edit source";
    document.getElementById("nmea-source-save").textContent = "Save source";
    document.getElementById("nmea-source-cancel").hidden = false;
    syncTransportFields();
    if (source.transport.type === "serial") scanPorts(source.transport.port);
    sourceName.focus();
  }

  function renderSources() {
    sourceList.replaceChildren();
    if (!sources.length) {
      const empty = document.createElement("p");
      empty.className = "settings-note";
      empty.textContent = "No NMEA sources configured.";
      sourceList.append(empty);
      renderArbitration();
      return;
    }
    for (const [index, source] of sources.entries()) {
      const row = document.createElement("div");
      row.className = "nmea-source-row";
      const toggle = document.createElement("input");
      toggle.type = "checkbox";
      toggle.checked = source.enabled;
      toggle.setAttribute("aria-label", `Enable ${source.name}`);
      toggle.addEventListener("change", () => {
        saveSources(sources.map((item) => item.id === source.id
          ? { ...item, enabled: toggle.checked, useForNavigation: toggle.checked && item.useForNavigation }
          : item));
      });
      const description = document.createElement("div");
      description.className = "nmea-source-description";
      const name = document.createElement("b");
      name.textContent = source.name;
      const detail = document.createElement("span");
      detail.className = "wx-note";
      const transport = source.transport;
      const endpoint = transport.type === "serial"
        ? `${transport.port} · ${transport.baudRate} baud`
        : transport.type === "tcp" ? transport.address : `UDP ${transport.bindAddress}`;
      const state = sourceStatuses.get(source.id);
      detail.classList.add("nmea-source-status");
      row.dataset.sourceId = source.id;
      detail.textContent = sourceStatusText(source, endpoint, state);
      description.append(name, detail);
      const edit = document.createElement("button");
      edit.type = "button";
      edit.textContent = "Edit";
      edit.addEventListener("click", () => editSource(source));
      const up = document.createElement("button");
      up.type = "button";
      up.textContent = "↑";
      up.title = "Move earlier in automatic fallback order";
      up.disabled = index === 0;
      up.addEventListener("click", () => reorderSource(index, index - 1));
      const down = document.createElement("button");
      down.type = "button";
      down.textContent = "↓";
      down.title = "Move later in automatic fallback order";
      down.disabled = index === sources.length - 1;
      down.addEventListener("click", () => reorderSource(index, index + 1));
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Remove";
      remove.addEventListener("click", () => {
        saveSources(sources.filter((item) => item.id !== source.id));
        if (editingId === source.id) resetSourceForm();
      });
      row.append(toggle, description, up, down, edit, remove);
      sourceList.append(row);
    }
    renderArbitration();
  }

  const GROUPS = [
    ["position", "Position"],
    ["motion", "COG / SOG"],
    ["heading", "Heading"],
    ["speedThroughWater", "Speed through water"],
    ["wind", "Wind"],
    ["depth", "Depth"],
    ["waterTemperature", "Water temperature"],
  ];

  function renderArbitration() {
    failoverCheckbox.checked = arbitration.failoverEnabled !== false;
    staleAfterSelect.value = String(arbitration.staleAfterSeconds || 10);
    arbitrationBox.replaceChildren();
    for (const [key, label] of GROUPS) {
      const wrapper = document.createElement("label");
      wrapper.textContent = `${label} source`;
      const select = document.createElement("select");
      select.dataset.group = key;
      select.add(new Option("Automatic (source order)", ""));
      for (const source of sources) {
        const enabledNote = source.enabled ? "" : " · disabled";
        select.add(new Option(`${source.name}${enabledNote}`, source.id));
      }
      select.value = arbitration.preferredSources?.[key] || "";
      select.addEventListener("change", () => {
        const preferredSources = { ...arbitration.preferredSources, [key]: select.value };
        if (!select.value) delete preferredSources[key];
        saveSources(sources, { ...arbitration, preferredSources });
      });
      wrapper.append(select);
      arbitrationBox.append(wrapper);
    }
  }

  async function reorderSource(from, to) {
    const next = [...sources];
    const [source] = next.splice(from, 1);
    next.splice(to, 0, source);
    await saveSources(next);
  }

  function sourceStatusText(source, endpoint, state) {
    const age = state?.lastSeen ? Date.now() - Date.parse(state.lastSeen) : Infinity;
    const stale = source.enabled && state?.state === "receiving"
      && age > Number(arbitration.staleAfterSeconds || 10) * 1000;
    const status = !source.enabled ? "disabled" : stale ? "stale" : state?.state || "starting";
    const detail = stale ? `No valid data for ${Math.round(age / 1000)} s` : state?.detail;
    return `${endpoint} · ${status}${state ? ` · ${state.validSentences} valid / ${state.invalidSentences} invalid${detail ? ` · ${detail}` : ""}` : ""}`;
  }

  function updateSourceStatus(payload) {
    const source = sources.find((item) => item.id === payload.id);
    if (!source) return;
    const row = [...sourceList.querySelectorAll(".nmea-source-row")].find((item) => item.dataset.sourceId === payload.id);
    const label = row?.querySelector(".nmea-source-status");
    if (!label) return;
    const transport = source.transport;
    const endpoint = transport.type === "serial"
      ? `${transport.port} · ${transport.baudRate} baud`
      : transport.type === "tcp" ? transport.address : `UDP ${transport.bindAddress}`;
    label.textContent = sourceStatusText(source, endpoint, payload);
  }

  function renderNmeaData() {
    const observations = Object.entries(activeGroupSources).flatMap(([group, id]) => {
      const observation = latestObservations.get(id);
      if (!observation) return [];
      const fields = {
        position: ["lat", "lon", "datetime"],
        motion: ["cog", "sog"],
        heading: ["heading", "headingMag", "variation"],
        speedThroughWater: ["stw"],
        wind: ["aws", "awa", "tws", "twa", "twd"],
        depth: ["depth", "depthKeel", "depthSurface"],
        waterTemperature: ["waterTemp"],
      }[group] || [];
      return [Object.fromEntries(fields.map((field) => [field, observation[field]]))];
    });
    const observation = Object.assign({}, ...observations);
    const labels = [
      ["Position", observation?.lat != null && observation?.lon != null
        ? `${observation.lat.toFixed(5)}, ${observation.lon.toFixed(5)}`
        : null],
      ["COG", observation?.cog != null ? `${observation.cog.toFixed(1)}°` : null],
      ["SOG", observation?.sog != null ? `${observation.sog.toFixed(2)} kn` : null],
      ["Heading", observation?.heading != null ? `${observation.heading.toFixed(1)}°T` : null],
      ["STW", observation?.stw != null ? `${observation.stw.toFixed(2)} kn` : null],
      ["Wind", observation?.aws != null ? `${observation.aws.toFixed(1)} kn @ ${observation.awa?.toFixed(0) ?? "—"}° apparent` : null],
      ["True wind", observation?.tws != null ? `${observation.tws.toFixed(1)} kn @ ${observation.twa?.toFixed(0) ?? observation.twd?.toFixed(0) ?? "—"}°` : null],
      ["Depth", observation?.depth != null ? `${observation.depth.toFixed(1)} m` : null],
      ["Water temp", observation?.waterTemp != null ? `${observation.waterTemp.toFixed(1)} °C` : null],
    ].filter(([, value]) => value != null);
    nmeaCurrentData.textContent = labels.length
      ? labels.map(([label, value]) => `${label} ${value}`).join(" · ")
      : "Waiting for supported data fields from enabled sources.";
    nmeaRawViewer.textContent = recentSentences.length
      ? recentSentences.join("\n")
      : "No NMEA sentences received yet.";
    nmeaRawViewer.scrollTop = nmeaRawViewer.scrollHeight;
  }

  async function saveSources(next, nextArbitration = arbitration) {
    const save = document.getElementById("nmea-source-save");
    save.disabled = true;
    showSourceStatus("Saving NMEA sources…");
    const ids = new Set(next.map((source) => source.id));
    nextArbitration = {
      ...nextArbitration,
      preferredSources: Object.fromEntries(
        Object.entries(nextArbitration.preferredSources || {}).filter(([, id]) => !id || ids.has(id)),
      ),
    };
    try {
      await invoke("nmea_sources_set", { sources: next, arbitration: nextArbitration });
      sources = next;
      arbitration = nextArbitration;
      if (saved) {
        saved.nmeaSources = next;
        saved.nmeaArbitration = nextArbitration;
      }
      sourceStatuses = new Map();
      renderSources();
      renderActiveSources();
      renderNmeaData();
      showSourceStatus("NMEA sources saved.", "good");
    } catch (error) {
      showSourceStatus(`Could not save NMEA sources: ${error}`, "bad");
      renderSources();
    } finally {
      save.disabled = false;
    }
  }

  sourceForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!sourceForm.reportValidity()) return;
    let transport;
    if (sourceType.value === "serial") {
      transport = { type: "serial", port: portSelect.value, baudRate: Number(sourceForm.elements.namedItem("baudRate").value) };
    } else if (sourceType.value === "tcp") {
      transport = { type: "tcp", address: sourceForm.elements.namedItem("address").value.trim() };
    } else {
      transport = { type: "udp", bindAddress: sourceForm.elements.namedItem("bindAddress").value.trim() };
    }
    const prior = sources.find((item) => item.id === editingId);
    const source = {
      id: editingId || `nmea-${crypto.randomUUID()}`,
      name: sourceName.value.trim(),
      enabled: prior?.enabled ?? true,
      useForNavigation: prior?.useForNavigation ?? !sources.some((item) => item.enabled && item.useForNavigation),
      transport,
    };
    const next = prior
      ? sources.map((item) => item.id === editingId ? source : item)
      : [...sources, source];
    await saveSources(next);
    if (sources.some((item) => item.id === source.id)) resetSourceForm();
  });
  document.getElementById("nmea-source-cancel").addEventListener("click", resetSourceForm);

  async function loadSources() {
    try {
      await scanPorts();
      const data = await invoke("nmea_sources_get");
      sources = data.sources || [];
      arbitration = data.arbitration || arbitration;
      sourceStatuses = new Map((data.statuses || []).map((item) => [item.id, item]));
      if (saved) {
        saved.nmeaSources = sources;
        saved.nmeaArbitration = arbitration;
      }
      renderSources();
      renderActiveSources();
      renderNmeaData();
    } catch (error) {
      showSourceStatus(`Could not load NMEA sources: ${error}`, "bad");
    }
  }
  listen("nmea-status", ({ payload }) => {
    if (payload?.id) {
      sourceStatuses.set(payload.id, payload);
      updateSourceStatus(payload);
    }
  }).catch((error) => showSourceStatus(`Could not subscribe to NMEA status: ${error}`, "bad"));
  // the sentences a source sent since the last event (about twice a second)
  listen("nmea-observation", ({ payload }) => {
    const raws = payload?.sentences ?? (payload?.sentence?.raw ? [payload.sentence.raw] : []);
    if (!raws.length || !payload.sourceId) return;
    latestObservations.set(payload.sourceId, payload.observation || {});
    const time = new Date().toLocaleTimeString();
    for (const raw of raws) recentSentences.push(`${time}  ${payload.sourceName || payload.sourceId}  ${raw}`);
    if (recentSentences.length > 100) recentSentences.splice(0, recentSentences.length - 100);
    renderNmeaData();
  }).catch((error) => showSourceStatus(`Could not subscribe to NMEA data: ${error}`, "bad"));
  function renderActiveSources() {
    const labels = Object.entries(activeGroupSources).map(([group, id]) => {
      const source = sources.find((item) => item.id === id);
      const title = GROUPS.find(([key]) => key === group)?.[1] || group;
      return `${title}: ${source?.name || id}`;
    });
    nmeaActiveSources.textContent = labels.length
      ? `Active sources — ${labels.join(" · ")}`
      : "No fresh source currently selected for any data group.";
  }

  listen("nmea-mux-status", ({ payload }) => {
    if (!payload) return;
    activeGroupSources = payload.groupSources || {};
    renderActiveSources();
    for (const state of sourceStatuses.values()) updateSourceStatus(state);
    renderNmeaData();
  }).catch((error) => showSourceStatus(`Could not subscribe to NMEA multiplexer status: ${error}`, "bad"));
  failoverCheckbox.addEventListener("change", () => {
    saveSources(sources, { ...arbitration, failoverEnabled: failoverCheckbox.checked });
  });
  staleAfterSelect.addEventListener("change", () => {
    saveSources(sources, { ...arbitration, staleAfterSeconds: Number(staleAfterSelect.value) });
  });

  saveButton.disabled = true;
  invoke("settings_get").then(async (settings) => {
    saved = settings;
    let profile = { ...EMPTY_PROFILE, ...(settings.vessel || {}) };
    if ((settings.schemaVersion || 0) < 1) {
      profile = { ...profile, ...profileFromLegacy(boat.get()) };
      const migrated = { ...settings, vessel: profile };
      try {
        await invoke("settings_save", { settings: migrated });
        saved = migrated;
      } catch (error) {
        showStatus(`Existing boat details loaded but could not be migrated: ${error}`, "bad");
      }
    }
    fill(profile);
    applyToBoat(profile);
    applyReferencePosition(profile);
    saveButton.disabled = false;
    await loadSources();
  }).catch((error) => {
    showStatus(`Could not load vessel settings: ${error}`, "bad");
  });
}
