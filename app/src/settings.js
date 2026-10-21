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
  const box = document.getElementById("settings");
  const button = document.getElementById("settings-btn");
  const form = document.getElementById("vessel-settings");
  const status = document.getElementById("settings-status");
  const saveButton = form.querySelector('button[type="submit"]');
  const sourceForm = document.getElementById("nmea-source-form");
  const sourceName = sourceForm.elements.namedItem("name");
  const sourceType = sourceForm.elements.namedItem("type");
  const portSelect = sourceForm.elements.namedItem("port");
  const sourceList = document.getElementById("nmea-sources");
  const sourceStatus = document.getElementById("nmea-settings-status");
  let saved = null;
  let sources = [];
  let sourceStatuses = new Map();
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
  function open(value) {
    box.hidden = !value;
    button.classList.toggle("on", value);
    if (value) {
      ui.open(null);
      form.querySelector("input")?.focus();
    }
    window.dispatchEvent(new Event("resize"));
  }

  button.addEventListener("click", () => open(box.hidden));
  document.getElementById("settings-close").addEventListener("click", () => open(false));
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !box.hidden) {
      event.preventDefault();
      open(false);
      button.focus();
    }
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

  async function scanPorts() {
    const selected = portSelect.value;
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
      portSelect.value = source.transport.port;
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
    if (source.transport.type === "serial" && !portSelect.value) scanPorts();
    sourceName.focus();
  }

  function renderSources() {
    sourceList.replaceChildren();
    if (!sources.length) {
      const empty = document.createElement("p");
      empty.className = "settings-note";
      empty.textContent = "No NMEA sources configured.";
      sourceList.append(empty);
      return;
    }
    for (const source of sources) {
      const row = document.createElement("div");
      row.className = "nmea-source-row";
      const toggle = document.createElement("input");
      toggle.type = "checkbox";
      toggle.checked = source.enabled;
      toggle.setAttribute("aria-label", `Enable ${source.name}`);
      toggle.addEventListener("change", () => {
        saveSources(sources.map((item) => item.id === source.id ? { ...item, enabled: toggle.checked } : item));
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
      detail.textContent = `${endpoint} · ${source.enabled ? (state?.state || "starting") : "disabled"}${state ? ` · ${state.validSentences} valid / ${state.invalidSentences} invalid${state.lastSentence ? ` · ${state.lastSentence}` : ""}${state.detail ? ` · ${state.detail}` : ""}` : ""}`;
      description.append(name, detail);
      const edit = document.createElement("button");
      edit.type = "button";
      edit.textContent = "Edit";
      edit.addEventListener("click", () => editSource(source));
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Remove";
      remove.addEventListener("click", () => {
        saveSources(sources.filter((item) => item.id !== source.id));
        if (editingId === source.id) resetSourceForm();
      });
      row.append(toggle, description, edit, remove);
      sourceList.append(row);
    }
  }

  async function saveSources(next) {
    const save = document.getElementById("nmea-source-save");
    save.disabled = true;
    showSourceStatus("Saving NMEA sources…");
    try {
      await invoke("nmea_sources_set", { sources: next });
      sources = next;
      if (saved) saved.nmeaSources = next;
      renderSources();
      showSourceStatus("NMEA sources saved. Received data is monitored but does not yet drive the chart.", "good");
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
      sourceStatuses = new Map((data.statuses || []).map((item) => [item.id, item]));
      if (saved) saved.nmeaSources = sources;
      renderSources();
    } catch (error) {
      showSourceStatus(`Could not load NMEA sources: ${error}`, "bad");
    }
  }
  listen("nmea-status", ({ payload }) => {
    if (payload?.id) {
      sourceStatuses.set(payload.id, payload);
      renderSources();
    }
  }).catch((error) => showSourceStatus(`Could not subscribe to NMEA status: ${error}`, "bad"));

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
