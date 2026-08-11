const tabHistory = document.getElementById("tab-history") as HTMLButtonElement;
const tabSettings = document.getElementById("tab-settings") as HTMLButtonElement;
const panelHistory = document.getElementById("panel-history") as HTMLElement;
const panelSettings = document.getElementById("panel-settings") as HTMLElement;

function activate(tab: "history" | "settings") {
  const isHistory = tab === "history";

  tabHistory.classList.toggle("tab--active", isHistory);
  tabHistory.setAttribute("aria-selected", String(isHistory));
  panelHistory.hidden = !isHistory;

  tabSettings.classList.toggle("tab--active", !isHistory);
  tabSettings.setAttribute("aria-selected", String(!isHistory));
  panelSettings.hidden = isHistory;
}

tabHistory.addEventListener("click", () => activate("history"));
tabSettings.addEventListener("click", () => activate("settings"));
