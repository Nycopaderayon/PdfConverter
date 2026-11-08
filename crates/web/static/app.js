const modes = [...document.querySelectorAll(".mode")];
const drop = document.getElementById("drop");
const fileInput = document.getElementById("file");
const dropText = document.getElementById("dropText");
const fileName = document.getElementById("fileName");
const convertButton = document.getElementById("convert");
const statusBox = document.getElementById("status");
const statusText = document.getElementById("statusText");
const barFill = document.getElementById("barFill");
const downloadLink = document.getElementById("download");
const errorBox = document.getElementById("error");
const fidelityBox = document.getElementById("fidelity");
const fidelityInputs = [...document.querySelectorAll('input[name="fidelity"]')];

const state = {
  target: "docx",
  file: null,
  jobId: null,
  timer: null,
};

// UI modes. `target` is the API target format; several modes share one target
// and differ only in the source files they accept.
const MODES = {
  docx: {
    target: "docx",
    extensions: [".pdf"],
    dropText: "Drop a PDF here, or click to browse",
    wrongFile: "Please choose a PDF file for this mode.",
  },
  pdf: {
    target: "pdf",
    extensions: [".docx", ".doc"],
    dropText: "Drop a Word document here, or click to browse",
    wrongFile: "Please choose a Word document (.docx or .doc) for this mode.",
  },
  wps: {
    target: "docx",
    extensions: [".wps", ".wpt"],
    dropText: "Drop a WPS document here, or click to browse",
    wrongFile: "Please choose a WPS document (.wps or .wpt) for this mode.",
  },
  wpspdf: {
    target: "pdf",
    extensions: [".wps", ".wpt"],
    dropText: "Drop a WPS document here, or click to browse",
    wrongFile: "Please choose a WPS document (.wps or .wpt) for this mode.",
  },
};

function isAccepted(file, mode) {
  const name = file.name.toLowerCase();
  return MODES[mode].extensions.some((extension) => name.endsWith(extension));
}

function setMode(mode) {
  state.target = mode;

  for (const button of modes) {
    const active = button.dataset.target === mode;
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-selected", String(active));
  }

  fileInput.accept = MODES[mode].extensions.join(",");
  dropText.textContent = MODES[mode].dropText;

  fidelityBox.hidden = mode !== "docx";

  if (state.file && !isAccepted(state.file, mode)) {
    setFile(null);
  }
}

function setFile(file) {
  state.file = file;
  fileName.textContent = file ? file.name : "";
  convertButton.disabled = !file;
  hideError();
  resetResult();
}

function resetResult() {
  if (state.timer) {
    clearTimeout(state.timer);
    state.timer = null;
  }
  state.jobId = null;
  statusBox.hidden = true;
  downloadLink.hidden = true;
  downloadLink.removeAttribute("href");
  barFill.classList.remove("is-indeterminate");
  barFill.style.width = "0";
  statusText.textContent = "";
}

function showStatus(message) {
  statusBox.hidden = false;
  statusText.textContent = message;
}

function showError(message) {
  if (state.timer) {
    clearTimeout(state.timer);
    state.timer = null;
  }
  errorBox.textContent = message;
  errorBox.hidden = false;
  statusBox.hidden = true;
  convertButton.disabled = !state.file;
  barFill.classList.remove("is-indeterminate");
}

function hideError() {
  errorBox.hidden = true;
  errorBox.textContent = "";
}

modes.forEach((mode) => {
  mode.addEventListener("click", () => setMode(mode.dataset.target));
});

drop.addEventListener("click", (event) => {
  if (event.target !== fileInput) {
    fileInput.click();
  }
});

fileInput.addEventListener("change", () => {
  setFile(fileInput.files && fileInput.files[0] ? fileInput.files[0] : null);
});

["dragenter", "dragover"].forEach((name) => {
  drop.addEventListener(name, (event) => {
    event.preventDefault();
    drop.classList.add("is-over");
  });
});

["dragleave", "drop"].forEach((name) => {
  drop.addEventListener(name, (event) => {
    event.preventDefault();
    drop.classList.remove("is-over");
  });
});

drop.addEventListener("drop", (event) => {
  const file = event.dataTransfer && event.dataTransfer.files[0];
  if (!file) return;

  if (!isAccepted(file, state.target)) {
    showError(MODES[state.target].wrongFile);
    return;
  }

  setFile(file);
});

convertButton.addEventListener("click", async () => {
  if (!state.file) return;

  hideError();
  resetResult();
  convertButton.disabled = true;
  barFill.classList.add("is-indeterminate");
  showStatus("Uploading…");

  const form = new FormData();
  form.append("file", state.file, state.file.name);
  form.append("target", MODES[state.target].target);

  if (state.target === "docx") {
    const selected = fidelityInputs.find((input) => input.checked);
    form.append("fidelity", selected ? selected.value : "editable");
  }

  try {
    const response = await fetch("/api/jobs", { method: "POST", body: form });
    const data = await response.json().catch(() => ({}));

    if (!response.ok) {
      throw new Error(data.error || `Upload failed (${response.status})`);
    }

    state.jobId = data.id;
    showStatus("Queued…");
    poll();
  } catch (error) {
    showError(error.message);
  }
});

async function poll() {
  if (!state.jobId) return;

  try {
    const response = await fetch(`/api/jobs/${state.jobId}`);
    if (!response.ok) {
      throw new Error("Could not read the conversion status.");
    }

    const job = await response.json();

    if (job.status === "queued") {
      showStatus("Queued…");
      state.timer = setTimeout(poll, 700);
      return;
    }

    if (job.status === "processing") {
      showStatus("Converting…");
      state.timer = setTimeout(poll, 700);
      return;
    }

    if (job.status === "done") {
      showStatus("Done");
      barFill.classList.remove("is-indeterminate");
      barFill.style.width = "100%";
      downloadLink.href = `/api/jobs/${state.jobId}/download`;
      downloadLink.setAttribute("download", job.output_name);
      downloadLink.hidden = false;
      convertButton.disabled = false;
      return;
    }

    if (job.status === "failed") {
      showError(job.error || "Conversion failed.");
      return;
    }

    state.timer = setTimeout(poll, 700);
  } catch (error) {
    showError(error.message);
  }
}

setMode("docx");
