<script lang="ts">
  import { api, type Finding, type DomainKey, DOMAIN_LABEL } from "../lib/api";
  import { bytesParts, idle, shortPath, bytes as fmtBytes } from "../lib/format";
  import { explainRisk } from "../lib/risk";

  let {
    findings,
    selected,
    home,
    domainFilter = null,
    onToggle,
    onToggleMany,
  }: {
    findings: Finding[];
    selected: Set<string>;
    home: string;
    domainFilter?: DomainKey | null;
    onToggle: (id: string) => void;
    onToggleMany: (ids: string[], on: boolean) => void;
  } = $props();

  let expanded = $state<string | null>(null);
  let searchQuery = $state("");
  let searchInputEl = $state<HTMLInputElement | null>(null);

  type SortKey = "bytes" | "days_idle" | "name" | "risk";
  type SortDir = "asc" | "desc";

  let sortKey = $state<SortKey>("bytes");
  let sortDir = $state<SortDir>("desc");
  let focusedIndex = $state<number>(-1);

  // Copy-to-clipboard state
  let copiedId = $state<string | null>(null);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  function copyText(text: string, id: string) {
    if (!text) return;
    navigator.clipboard
      .writeText(text)
      .then(() => {
        copiedId = id;
        if (copyTimer) clearTimeout(copyTimer);
        copyTimer = setTimeout(() => {
          copiedId = null;
        }, 1800);
      })
      .catch(() => {
        // Fallback for non-secure contexts
        copiedId = id;
        if (copyTimer) clearTimeout(copyTimer);
        copyTimer = setTimeout(() => {
          copiedId = null;
        }, 1800);
      });
  }

  // Docker sub-navigation
  type DockerTab = "all" | "images" | "containers" | "volumes" | "build_cache";
  let dockerTab = $state<DockerTab>("all");
  type ImageSubFilter = "all" | "pullable" | "local" | "untagged";
  let imageSubFilter = $state<ImageSubFilter>("all");

  // Project view mode
  type ViewMode = "items" | "projects";
  let viewMode = $state<ViewMode>("items");

  const isDocker = $derived(
    domainFilter === "docker" ||
      (findings.length > 0 && findings.every((f) => f.domain === "docker"))
  );

  function getDockerType(
    f: Finding
  ): "image" | "container" | "volume" | "build_cache" | "vm_disk" | null {
    if (f.domain !== "docker") return null;
    const meta = f.meta as Record<string, unknown> | undefined;
    const dt = meta?.docker_type;
    if (dt === "image" || f.id.startsWith("docker:image:")) return "image";
    if (dt === "container" || f.id.startsWith("docker:container:")) return "container";
    if (dt === "volume" || f.id.startsWith("docker:volume:")) return "volume";
    if (dt === "build_cache" || f.id === "docker:build_cache") return "build_cache";
    if (f.id === "docker:vm_disk") return "vm_disk";
    return null;
  }

  const dockerCounts = $derived.by(() => {
    let all = 0,
      images = 0,
      containers = 0,
      volumes = 0,
      cache = 0;
    let unusedImages = 0,
      inUseImages = 0;
    let pullableImages = 0,
      localBuildImages = 0,
      untaggedImages = 0;
    let pullableUnused = 0,
      localBuildUnused = 0;
    let stoppedContainers = 0,
      activeContainers = 0;
    let danglingVolumes = 0,
      attachedVolumes = 0;

    for (const f of findings) {
      if (f.domain !== "docker") continue;
      all++;
      const t = getDockerType(f);
      const meta = f.meta as Record<string, unknown> | undefined;
      if (t === "image") {
        images++;
        const isPull = meta?.is_pullable === true;
        const origin = meta?.origin as string | undefined;
        if (isPull) {
          pullableImages++;
          if (f.risk === "caution") pullableUnused++;
        } else if (origin === "local_build") {
          localBuildImages++;
          if (f.risk === "caution") localBuildUnused++;
        } else if (origin === "dangling_build") {
          untaggedImages++;
        }
        if (f.risk === "caution") unusedImages++;
        else if (f.risk === "advisory") inUseImages++;
      } else if (t === "container") {
        containers++;
        if (f.risk === "caution") stoppedContainers++;
        else if (f.risk === "advisory") activeContainers++;
      } else if (t === "volume") {
        volumes++;
        if (f.risk === "caution") danglingVolumes++;
        else if (f.risk === "advisory") attachedVolumes++;
      } else if (t === "build_cache") {
        cache++;
      }
    }
    return {
      all,
      images,
      containers,
      volumes,
      cache,
      unusedImages,
      inUseImages,
      pullableImages,
      localBuildImages,
      untaggedImages,
      pullableUnused,
      localBuildUnused,
      stoppedContainers,
      activeContainers,
      danglingVolumes,
      attachedVolumes,
    };
  });

  // Selection collections for electability
  const dockerPullableUnusedIds = $derived(
    findings
      .filter((f) => {
        if (f.domain !== "docker" || getDockerType(f) !== "image" || f.risk !== "caution") {
          return false;
        }
        const meta = f.meta as Record<string, unknown> | undefined;
        return meta?.is_pullable === true;
      })
      .map((f) => f.id)
  );
  const allPullableUnusedSelected = $derived(
    dockerPullableUnusedIds.length > 0 &&
      dockerPullableUnusedIds.every((id) => selected.has(id))
  );

  const dockerLocalBuildUnusedIds = $derived(
    findings
      .filter((f) => {
        if (f.domain !== "docker" || getDockerType(f) !== "image" || f.risk !== "caution") {
          return false;
        }
        const meta = f.meta as Record<string, unknown> | undefined;
        return meta?.origin === "local_build";
      })
      .map((f) => f.id)
  );
  const allLocalBuildUnusedSelected = $derived(
    dockerLocalBuildUnusedIds.length > 0 &&
      dockerLocalBuildUnusedIds.every((id) => selected.has(id))
  );

  const dockerUnusedImageIds = $derived(
    findings
      .filter((f) => f.domain === "docker" && getDockerType(f) === "image" && f.risk === "caution")
      .map((f) => f.id)
  );
  const allUnusedImagesSelected = $derived(
    dockerUnusedImageIds.length > 0 &&
      dockerUnusedImageIds.every((id) => selected.has(id))
  );

  const dockerStoppedContainerIds = $derived(
    findings
      .filter(
        (f) => f.domain === "docker" && getDockerType(f) === "container" && f.risk === "caution"
      )
      .map((f) => f.id)
  );
  const allStoppedContainersSelected = $derived(
    dockerStoppedContainerIds.length > 0 &&
      dockerStoppedContainerIds.every((id) => selected.has(id))
  );

  const dockerDanglingVolumeIds = $derived(
    findings
      .filter((f) => f.domain === "docker" && getDockerType(f) === "volume" && f.risk === "caution")
      .map((f) => f.id)
  );
  const allDanglingVolumesSelected = $derived(
    dockerDanglingVolumeIds.length > 0 &&
      dockerDanglingVolumeIds.every((id) => selected.has(id))
  );

  const dockerCacheIds = $derived(
    findings
      .filter((f) => f.domain === "docker" && getDockerType(f) === "build_cache")
      .map((f) => f.id)
  );
  const allDockerCacheSelected = $derived(
    dockerCacheIds.length > 0 && dockerCacheIds.every((id) => selected.has(id))
  );

  const dockerReclaimableIds = $derived(
    findings
      .filter((f) => f.domain === "docker" && f.risk !== "advisory")
      .map((f) => f.id)
  );
  const allDockerReclaimableSelected = $derived(
    dockerReclaimableIds.length > 0 &&
      dockerReclaimableIds.every((id) => selected.has(id))
  );

  function togglePullableImages() {
    onToggleMany(dockerPullableUnusedIds, !allPullableUnusedSelected);
  }
  function toggleLocalBuildImages() {
    onToggleMany(dockerLocalBuildUnusedIds, !allLocalBuildUnusedSelected);
  }
  function toggleUnusedImages() {
    onToggleMany(dockerUnusedImageIds, !allUnusedImagesSelected);
  }
  function toggleStoppedContainers() {
    onToggleMany(dockerStoppedContainerIds, !allStoppedContainersSelected);
  }
  function toggleDanglingVolumes() {
    onToggleMany(dockerDanglingVolumeIds, !allDanglingVolumesSelected);
  }
  function toggleDockerCache() {
    onToggleMany(dockerCacheIds, !allDockerCacheSelected);
  }
  function toggleDockerReclaimable() {
    onToggleMany(dockerReclaimableIds, !allDockerReclaimableSelected);
  }

  function setSort(key: SortKey) {
    if (sortKey === key) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortKey = key;
      sortDir = key === "name" || key === "risk" ? "asc" : "desc";
    }
  }

  const filteredFindings = $derived.by(() => {
    let list = findings;

    // Filter by Docker sub-category if viewing Docker
    if (isDocker && dockerTab !== "all") {
      list = list.filter((f) => {
        const t = getDockerType(f);
        if (dockerTab === "images") {
          if (t !== "image") return false;
          const meta = f.meta as Record<string, unknown> | undefined;
          if (imageSubFilter === "pullable") return meta?.is_pullable === true;
          if (imageSubFilter === "local") return meta?.origin === "local_build";
          if (imageSubFilter === "untagged") return meta?.origin === "dangling_build";
          return true;
        }
        if (dockerTab === "containers") return t === "container";
        if (dockerTab === "volumes") return t === "volume";
        if (dockerTab === "build_cache") return t === "build_cache";
        return true;
      });
    }

    const q = searchQuery.trim().toLowerCase();
    if (q) {
      list = list.filter(
        (f) =>
          f.name.toLowerCase().includes(q) ||
          f.detail.toLowerCase().includes(q) ||
          (f.path && f.path.toLowerCase().includes(q))
      );
    }
    return [...list].sort((a, b) => {
      let cmp = 0;
      if (sortKey === "bytes") {
        cmp = a.bytes - b.bytes;
      } else if (sortKey === "days_idle") {
        cmp = a.days_idle - b.days_idle;
      } else if (sortKey === "name") {
        cmp = a.name.localeCompare(b.name);
      } else if (sortKey === "risk") {
        const riskOrder: Record<string, number> = { safe: 0, caution: 1, advisory: 2 };
        cmp = (riskOrder[a.risk] ?? 9) - (riskOrder[b.risk] ?? 9);
      }
      return sortDir === "asc" ? cmp : -cmp;
    });
  });

  interface ProjectGroup {
    projectDir: string;
    projectName: string;
    gitIdleDays: number | null;
    items: Finding[];
    totalBytes: number;
    allSelected: boolean;
    someSelected: boolean;
  }

  const projectGroups = $derived.by(() => {
    const map = new Map<
      string,
      {
        projectDir: string;
        projectName: string;
        gitIdleDays: number | null;
        items: Finding[];
        totalBytes: number;
      }
    >();

    for (const f of filteredFindings) {
      if (
        f.domain !== "node_modules" &&
        f.domain !== "venv" &&
        f.domain !== "artifact"
      ) {
        continue;
      }
      const meta = (f.meta ?? {}) as Record<string, any>;
      const pdir =
        meta.project_dir ||
        (f.path ? f.path.substring(0, f.path.lastIndexOf("/")) : null);
      if (!pdir) continue;

      const pname =
        meta.project_name || pdir.substring(pdir.lastIndexOf("/") + 1);
      const gitIdle =
        typeof meta.git_idle_days === "number" ? meta.git_idle_days : null;

      let group = map.get(pdir);
      if (!group) {
        group = {
          projectDir: pdir,
          projectName: pname,
          gitIdleDays: gitIdle,
          items: [],
          totalBytes: 0,
        };
        map.set(pdir, group);
      }
      group.items.push(f);
      group.totalBytes += f.bytes;
      if (
        gitIdle !== null &&
        (group.gitIdleDays === null || gitIdle < group.gitIdleDays)
      ) {
        group.gitIdleDays = gitIdle;
      }
    }

    const groups: ProjectGroup[] = [];
    for (const g of map.values()) {
      const selectableItems = g.items.filter((item) => item.risk !== "advisory");
      const allSelected =
        selectableItems.length > 0 &&
        selectableItems.every((item) => selected.has(item.id));
      const someSelected =
        selectableItems.some((item) => selected.has(item.id)) && !allSelected;
      groups.push({
        ...g,
        allSelected,
        someSelected,
      });
    }

    return groups.sort((a, b) => {
      const aDays = a.gitIdleDays ?? 0;
      const bDays = b.gitIdleDays ?? 0;
      if (bDays !== aDays) return bDays - aDays;
      return b.totalBytes - a.totalBytes;
    });
  });

  const staleProjectIds = $derived(
    projectGroups
      .filter((g) => (g.gitIdleDays ?? 0) >= 14)
      .flatMap((g) =>
        g.items.filter((i) => i.risk !== "advisory").map((i) => i.id)
      )
  );
  const allStaleSelected = $derived(
    staleProjectIds.length > 0 &&
      staleProjectIds.every((id) => selected.has(id))
  );

  function toggleProject(g: ProjectGroup) {
    const ids = g.items
      .filter((item) => item.risk !== "advisory")
      .map((item) => item.id);
    onToggleMany(ids, !g.allSelected);
  }

  function toggleStaleProjects() {
    onToggleMany(staleProjectIds, !allStaleSelected);
  }

  // Advisory findings are reported, never deleted, so they can't be selected and
  // must not count toward "all selected".
  const selectable = $derived(filteredFindings.filter((f) => f.risk !== "advisory"));
  const allOn = $derived(
    selectable.length > 0 && selectable.every((f) => selected.has(f.id))
  );
  const someOn = $derived(selectable.some((f) => selected.has(f.id)) && !allOn);

  function toggleAll() {
    onToggleMany(
      selectable.map((f) => f.id),
      !allOn
    );
  }

  const safeFindings = $derived(filteredFindings.filter((f) => f.risk === "safe"));
  const allSafeOn = $derived(
    safeFindings.length > 0 && safeFindings.every((f) => selected.has(f.id))
  );
  function toggleSafe() {
    onToggleMany(
      safeFindings.map((f) => f.id),
      !allSafeOn
    );
  }

  // Tooltip
  let tip = $state<{
    x: number;
    y: number;
    meaning: string;
    reason: string;
    risk: string;
  } | null>(null);

  function showTip(e: Event, f: Finding) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const w = 320;
    tip = {
      x: Math.max(8, Math.min(r.left + r.width / 2 - w / 2, window.innerWidth - w - 8)),
      y: r.bottom + 6,
      risk: f.risk,
      ...explainRisk(f),
    };
  }

  // Row windowing: fixed row height of 56px keeps thousands of findings smooth.
  const ROW_HEIGHT = 56;
  const OVERSCAN = 8;
  let bodyEl = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let clientHeight = $state(600);

  function onScroll() {
    if (!bodyEl) return;
    scrollTop = bodyEl.scrollTop;
    clientHeight = bodyEl.clientHeight;
  }

  const totalRows = $derived(filteredFindings.length);
  const isWindowed = $derived(totalRows >= 50 && expanded === null);

  const startIndex = $derived(
    !isWindowed ? 0 : Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
  );
  const endIndex = $derived(
    !isWindowed
      ? totalRows
      : Math.min(totalRows, Math.ceil((scrollTop + clientHeight) / ROW_HEIGHT) + OVERSCAN)
  );
  const visibleFindings = $derived(
    !isWindowed ? filteredFindings : filteredFindings.slice(startIndex, endIndex)
  );
  const topSpacer = $derived(!isWindowed ? 0 : startIndex * ROW_HEIGHT);
  const bottomSpacer = $derived(
    !isWindowed ? 0 : (totalRows - endIndex) * ROW_HEIGHT
  );

  function scrollFocusedIntoView() {
    if (!bodyEl || focusedIndex < 0) return;
    const targetTop = focusedIndex * ROW_HEIGHT;
    if (targetTop < bodyEl.scrollTop) {
      bodyEl.scrollTop = targetTop;
    } else if (targetTop + ROW_HEIGHT > bodyEl.scrollTop + clientHeight) {
      bodyEl.scrollTop = targetTop + ROW_HEIGHT - clientHeight;
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const isInput =
      target?.tagName === "INPUT" && target?.getAttribute("type") !== "checkbox";

    if (isInput) {
      if (e.key === "Escape") {
        searchQuery = "";
        (target as HTMLInputElement).blur();
      }
      return;
    }

    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchInputEl?.focus();
      return;
    }

    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      toggleAll();
      return;
    }

    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (filteredFindings.length === 0) return;
      focusedIndex = Math.min(focusedIndex + 1, filteredFindings.length - 1);
      scrollFocusedIntoView();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (filteredFindings.length === 0) return;
      focusedIndex = Math.max(focusedIndex - 1, 0);
      scrollFocusedIntoView();
    } else if (e.key === " ") {
      if (focusedIndex >= 0 && focusedIndex < filteredFindings.length) {
        const f = filteredFindings[focusedIndex];
        if (f.risk !== "advisory") {
          e.preventDefault();
          onToggle(f.id);
        }
      }
    } else if (e.key === "Enter") {
      if (focusedIndex >= 0 && focusedIndex < filteredFindings.length) {
        e.preventDefault();
        const f = filteredFindings[focusedIndex];
        expanded = expanded === f.id ? null : f.id;
      }
    } else if (e.key === "Escape") {
      if (expanded) {
        e.preventDefault();
        expanded = null;
      } else if (searchQuery) {
        e.preventDefault();
        searchQuery = "";
      }
    }
  }

  // Presentation helpers for Craft Standard
  function getOriginInfo(
    f: Finding,
    dockerType: ReturnType<typeof getDockerType>,
    meta: Record<string, any>
  ): { label: string; cls?: string } {
    if (dockerType === "container") return { label: "container" };
    if (dockerType === "volume")
      return { label: meta.driver ? `vol (${meta.driver})` : "volume" };
    if (dockerType === "build_cache") return { label: "buildkit" };
    if (dockerType === "vm_disk") return { label: "vm disk" };
    if (dockerType === "image") {
      if (meta.is_pullable) return { label: "registry", cls: "origin-registry" };
      if (meta.origin === "local_build")
        return { label: "local build", cls: "origin-local" };
      if (meta.origin === "dangling_build")
        return { label: "untagged", cls: "origin-untagged" };
      return { label: "image" };
    }
    if (f.id.startsWith("sim:") || meta.udid) {
      return { label: "simulator", cls: "origin-sim" };
    }
    if (f.domain === "large_file") {
      const ext =
        meta.category || (f.path ? f.path.split(".").pop() : "file");
      return { label: String(ext).toLowerCase() };
    }
    if (f.domain === "venv") return { label: "python venv" };
    if (f.domain === "node_modules") return { label: "node dep" };
    if (f.domain === "ai_model") return { label: "ai model" };
    if (f.domain === "cache") return { label: "cache" };
    if (f.domain === "artifact") return { label: "artifact" };
    return { label: DOMAIN_LABEL[f.domain] ?? f.domain };
  }

  function getStatusInfo(
    f: Finding,
    dockerType: ReturnType<typeof getDockerType>,
    meta: Record<string, any>
  ): { dot?: "active" | "stopped" | "dangling" | "unused"; label: string; isIdle?: boolean } {
    if (dockerType === "container") {
      if (meta.state === "running") return { dot: "active", label: "running" };
      return { dot: "stopped", label: "stopped" };
    }
    if (dockerType === "volume") {
      if (meta.is_active) return { dot: "active", label: "in use" };
      return { dot: "dangling", label: "dangling" };
    }
    if (dockerType === "image") {
      if (meta.containers > 0) return { dot: "active", label: "in use" };
      return { dot: "unused", label: "unused" };
    }
    if (dockerType === "build_cache") {
      return { dot: "unused", label: "ready" };
    }
    if (dockerType === "vm_disk") {
      return { dot: "active", label: "sparse" };
    }
    if (f.id.startsWith("sim:") || meta.udid) {
      if (meta.state === "Booted") return { dot: "active", label: "booted" };
      if (meta.is_available === false) return { dot: "unused", label: "unavailable" };
      return { dot: "stopped", label: "shutdown" };
    }
    return { label: idle(f.days_idle), isIdle: true };
  }

  function getResourceId(
    f: Finding,
    dockerType: ReturnType<typeof getDockerType>,
    meta: Record<string, any>
  ): string {
    if (meta.udid) return String(meta.udid);
    if (dockerType === "container") return meta.container_id || f.id;
    if (dockerType === "image") return meta.image_id || f.id;
    if (dockerType === "volume") return meta.volume_name || f.name;
    return f.path || f.id;
  }

  function getSafetyPolicyText(
    f: Finding,
    dockerType: ReturnType<typeof getDockerType>,
    meta: Record<string, any>
  ): string {
    if (f.id.startsWith("sim:") || meta.udid) {
      if (meta.state === "Booted") {
        return "Protected (Advisory) — Simulator device is currently booted and running. Woosh protects active simulators from deletion.";
      }
      if (meta.is_available === false) {
        return "Safe — Orphaned simulator. The runtime for this device is no longer installed on this Mac. Removing it reclaims storage without breaking Xcode.";
      }
      return "Caution — Valid simulator device (Shutdown). Reclaiming removes the device data. It can be recreated in Xcode or via simctl if needed.";
    }
    if (dockerType === "container") {
      if (meta.state === "running") {
        return "Protected (Advisory) — Container is actively running. Woosh prevents deletion to avoid data loss. Stop the container first (`docker stop`).";
      }
      return "Caution — Stopped container. Reclaiming removes the writable layer (SizeRw). Associated named volumes are preserved unless individually selected.";
    }
    if (dockerType === "volume") {
      if (meta.is_active) {
        return "Protected (Advisory) — Volume is actively attached to running container(s). Woosh will never delete it.";
      }
      return "Caution — Dangling volume. Reclaiming frees disk space, but volume data cannot be recovered once removed.";
    }
    if (dockerType === "image") {
      if (meta.containers > 0) {
        return "Protected (Advisory) — Image is currently in use by one or more containers. Woosh protects referenced images from deletion.";
      }
      if (meta.is_pullable) {
        return "Safe / Caution — Verified registry image. Reclaiming is safe because the image can be re-pulled at any time.";
      }
      if (meta.origin === "local_build") {
        return "Caution — Built locally from Dockerfile. If deleted, this image cannot be pulled from a registry and must be rebuilt from local source code.";
      }
      return "Safe — Untagged intermediate build stage left over from previous builds. Safe to reclaim.";
    }
    if (dockerType === "build_cache") {
      return "Safe — BuildKit cache. Intermediate layers are recomputed or re-downloaded automatically during subsequent builds.";
    }
    if (f.domain === "large_file") {
      return "Caution — User file. Woosh moves it to macOS Trash; Put Back in Finder restores it to original location.";
    }
    if (f.risk === "safe") {
      return "Safe — Regenerated automatically by local toolchains without network downloads.";
    }
    if (f.risk === "advisory") {
      return "Advisory — Reported for situational awareness only. Woosh will not delete this item.";
    }
    return "Caution — Recoverable, but may incur download bandwidth or compilation time to restore.";
  }

  function getCommandSnippet(
    f: Finding,
    dockerType: ReturnType<typeof getDockerType>,
    meta: Record<string, any>
  ): { label: string; snippet: string } | null {
    if (dockerType === "container") {
      if (meta.state === "running") {
        return {
          label: "Stop Container",
          snippet: `docker stop ${meta.name || meta.container_id}`,
        };
      }
      return {
        label: "Start Container",
        snippet: `docker start ${meta.name || meta.container_id}`,
      };
    }
    if (dockerType === "image") {
      if (meta.is_pullable && (meta.tag || f.name)) {
        return {
          label: "Pull Image",
          snippet: `docker pull ${meta.tag || f.name}`,
        };
      }
      if (meta.origin === "local_build") {
        const tag = meta.tag || f.name;
        return {
          label: "Rebuild Image",
          snippet: meta.compose_project
            ? "docker compose build"
            : `docker build -t ${tag} .`,
        };
      }
      if (meta.origin === "dangling_build") {
        return {
          label: "Remove Layer",
          snippet: `docker rmi ${meta.image_id || f.id}`,
        };
      }
    }
    if (dockerType === "volume") {
      return {
        label: "Inspect Volume",
        snippet: `docker volume inspect ${meta.volume_name || f.name}`,
      };
    }
    if (f.id.startsWith("sim:") || meta.udid) {
      if (meta.is_available === false) {
        return {
          label: "Delete Unavailable",
          snippet: "xcrun simctl delete unavailable",
        };
      }
      if (f.restore) {
        return {
          label: "Recreate Simulator",
          snippet: f.restore,
        };
      }
    }
    if (f.restore) {
      return { label: "Restore Command", snippet: f.restore };
    }
    if (f.domain === "large_file" && f.path) {
      return { label: "Quick Look", snippet: `qlmanage -p "${f.path}"` };
    }
    return null;
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<div class="table">
  <!-- Unified 36px macOS Command & Segmented Navigation Bar -->
  <div class="unified-bar">
    <div class="bar-left">
      {#if isDocker}
        <div class="segmented-control" role="tablist">
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={dockerTab === "all"}
            onclick={() => (dockerTab = "all")}
          >
            All
            <span class="segment-badge mono">{dockerCounts.all}</span>
          </button>
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={dockerTab === "images"}
            onclick={() => {
              dockerTab = "images";
              imageSubFilter = "all";
            }}
          >
            Images
            <span class="segment-badge mono">{dockerCounts.images}</span>
          </button>
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={dockerTab === "containers"}
            onclick={() => (dockerTab = "containers")}
          >
            Containers
            <span class="segment-badge mono">{dockerCounts.containers}</span>
          </button>
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={dockerTab === "volumes"}
            onclick={() => (dockerTab = "volumes")}
          >
            Volumes
            <span class="segment-badge mono">{dockerCounts.volumes}</span>
          </button>
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={dockerTab === "build_cache"}
            onclick={() => (dockerTab = "build_cache")}
          >
            Cache
            <span class="segment-badge mono">{dockerCounts.cache}</span>
          </button>
        </div>

        {#if dockerTab === "images"}
          <div class="sub-segmented-control" role="tablist">
            <button
              type="button"
              class="sub-segment"
              class:active={imageSubFilter === "all"}
              onclick={() => (imageSubFilter = "all")}
            >
              All
            </button>
            <button
              type="button"
              class="sub-segment"
              class:active={imageSubFilter === "pullable"}
              onclick={() => (imageSubFilter = "pullable")}
              title="Registry images (pullable anytime)"
            >
              Registry ({dockerCounts.pullableImages})
            </button>
            <button
              type="button"
              class="sub-segment"
              class:active={imageSubFilter === "local"}
              onclick={() => (imageSubFilter = "local")}
              title="Images built on this device"
            >
              Local ({dockerCounts.localBuildImages})
            </button>
            {#if dockerCounts.untaggedImages > 0}
              <button
                type="button"
                class="sub-segment"
                class:active={imageSubFilter === "untagged"}
                onclick={() => (imageSubFilter = "untagged")}
                title="Untagged intermediate build stages"
              >
                Untagged ({dockerCounts.untaggedImages})
              </button>
            {/if}
          </div>
        {/if}
      {/if}

      {#if !isDocker && projectGroups.length > 0}
        <div class="segmented-control" role="tablist">
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={viewMode === "items"}
            onclick={() => (viewMode = "items")}
          >
            All Items
            <span class="segment-badge mono">{filteredFindings.length}</span>
          </button>
          <button
            type="button"
            role="tab"
            class="segment"
            class:active={viewMode === "projects"}
            onclick={() => (viewMode = "projects")}
          >
            Stale Projects
            <span class="segment-badge mono">{projectGroups.length}</span>
          </button>
        </div>
      {/if}

      <!-- Search Box with ⌘F -->
      <div class="search-box">
        <span class="search-glyph">⌕</span>
        <input
          bind:this={searchInputEl}
          type="search"
          class="search-input mono"
          placeholder="Filter (⌘F)..."
          bind:value={searchQuery}
        />
        {#if searchQuery}
          <button
            type="button"
            class="clear-btn"
            onclick={() => (searchQuery = "")}
            title="Clear filter (Esc)">Clear</button
          >
        {/if}
      </div>
    </div>

    <!-- Right Side Contextual Action -->
    <div class="bar-actions">
      {#if isDocker}
        {#if dockerTab === "all" && dockerReclaimableIds.length > 0}
          <button
            type="button"
            class="action-btn"
            class:active={allDockerReclaimableSelected}
            onclick={toggleDockerReclaimable}
            title="Toggle selection for all reclaimable Docker items"
          >
            <span>{allDockerReclaimableSelected ? "Deselect Reclaimable" : "Select Reclaimable"}</span>
            <span class="mono">({dockerReclaimableIds.length})</span>
          </button>
        {:else if dockerTab === "images"}
          {#if imageSubFilter === "pullable" && dockerPullableUnusedIds.length > 0}
            <button
              type="button"
              class="action-btn"
              class:active={allPullableUnusedSelected}
              onclick={togglePullableImages}
              title="Select unused images pullable from registry"
            >
              <span>{allPullableUnusedSelected ? "Deselect Pullable" : "Select Pullable Unused"}</span>
              <span class="mono">({dockerPullableUnusedIds.length})</span>
            </button>
          {:else if imageSubFilter === "local" && dockerLocalBuildUnusedIds.length > 0}
            <button
              type="button"
              class="action-btn"
              class:active={allLocalBuildUnusedSelected}
              onclick={toggleLocalBuildImages}
              title="Select unused images built locally"
            >
              <span>{allLocalBuildUnusedSelected ? "Deselect Local" : "Select Local Builds"}</span>
              <span class="mono">({dockerLocalBuildUnusedIds.length})</span>
            </button>
          {:else if dockerUnusedImageIds.length > 0}
            <button
              type="button"
              class="action-btn"
              class:active={allUnusedImagesSelected}
              onclick={toggleUnusedImages}
              title="Select all unused images"
            >
              <span>{allUnusedImagesSelected ? "Deselect Unused" : "Select Unused"}</span>
              <span class="mono">({dockerUnusedImageIds.length})</span>
            </button>
          {/if}
        {:else if dockerTab === "containers" && dockerStoppedContainerIds.length > 0}
          <button
            type="button"
            class="action-btn"
            class:active={allStoppedContainersSelected}
            onclick={toggleStoppedContainers}
            title="Select all stopped containers (active containers are protected)"
          >
            <span>{allStoppedContainersSelected ? "Deselect Stopped" : "Select Stopped"}</span>
            <span class="mono">({dockerStoppedContainerIds.length})</span>
          </button>
        {:else if dockerTab === "volumes" && dockerDanglingVolumeIds.length > 0}
          <button
            type="button"
            class="action-btn"
            class:active={allDanglingVolumesSelected}
            onclick={toggleDanglingVolumes}
            title="Select all dangling/unattached volumes"
          >
            <span>{allDanglingVolumesSelected ? "Deselect Dangling" : "Select Dangling"}</span>
            <span class="mono">({dockerDanglingVolumeIds.length})</span>
          </button>
        {:else if dockerTab === "build_cache" && dockerCacheIds.length > 0}
          <button
            type="button"
            class="action-btn"
            class:active={allDockerCacheSelected}
            onclick={toggleDockerCache}
            title="Select Docker BuildKit cache"
          >
            <span>{allDockerCacheSelected ? "Deselect Cache" : "Select Build Cache"}</span>
          </button>
        {/if}
      {:else if viewMode === "projects"}
        {#if staleProjectIds.length > 0}
          <button
            type="button"
            class="action-btn"
            class:active={allStaleSelected}
            onclick={toggleStaleProjects}
            title="Select all regenerable deps from projects idle > 14 days"
          >
            <span>{allStaleSelected ? "Deselect Stale (>14d)" : "Select Stale (>14d)"}</span>
            <span class="mono">({staleProjectIds.length})</span>
          </button>
        {/if}
        <span class="item-count mono faint">
          {projectGroups.length} {projectGroups.length === 1 ? "project" : "projects"}
        </span>
      {:else if safeFindings.length > 0}
        <button
          type="button"
          class="action-btn"
          class:active={allSafeOn}
          onclick={toggleSafe}
          title="Toggle selection for all Safe items"
        >
          <span>{allSafeOn ? "Deselect Safe" : "Select Safe"}</span>
          <span class="mono">({safeFindings.length})</span>
        </button>
      {/if}

      {#if isDocker || viewMode !== "projects"}
        <span class="item-count mono faint">
          {filteredFindings.length} {filteredFindings.length === 1 ? "item" : "items"}
        </span>
      {/if}
    </div>
  </div>

  <!-- Strict Column Table Header -->
  <div class="head row">
    <div class="c-check">
      <input
        type="checkbox"
        checked={allOn}
        indeterminate={someOn}
        onchange={toggleAll}
        aria-label="Select all"
      />
    </div>
    <button type="button" class="c-item col-btn" onclick={() => setSort("name")}>
      <span>{viewMode === "projects" ? "Project" : "Item"}</span>
      <span class="sort-icon">{sortKey === "name" ? (sortDir === "asc" ? "▲" : "▼") : ""}</span>
    </button>
    <div class="c-origin col-hdr">
      <span>{viewMode === "projects" ? "Dependencies" : "Origin"}</span>
    </div>
    <button type="button" class="c-status col-btn" onclick={() => setSort("days_idle")}>
      <span>{viewMode === "projects" ? "Git Activity" : "Status"}</span>
      <span class="sort-icon">{sortKey === "days_idle" ? (sortDir === "asc" ? "▲" : "▼") : ""}</span>
    </button>
    <button type="button" class="c-risk col-btn" onclick={() => setSort("risk")}>
      <span>Risk</span>
      <span class="sort-icon">{sortKey === "risk" ? (sortDir === "asc" ? "▲" : "▼") : ""}</span>
    </button>
    <button type="button" class="c-size col-btn sort-right" onclick={() => setSort("bytes")}>
      <span>Reclaims</span>
      <span class="sort-icon">{sortKey === "bytes" ? (sortDir === "asc" ? "▲" : "▼") : ""}</span>
    </button>
  </div>

  <!-- Table Body with Virtual Scrolling -->
  <div class="body" bind:this={bodyEl} onscroll={onScroll}>
    {#if viewMode === "projects"}
      {#each projectGroups as g (g.projectDir)}
        {@const [num, unit] = bytesParts(g.totalBytes)}
        {@const hasExpanded = expanded === `proj:${g.projectDir}`}

        <div
          class="row item"
          class:on={g.allSelected}
          class:is-expanded={hasExpanded}
          role="presentation"
        >
          <div class="c-check">
            <input
              type="checkbox"
              checked={g.allSelected}
              indeterminate={g.someSelected}
              onchange={() => toggleProject(g)}
              aria-label={`Select ${g.projectName}`}
            />
          </div>

          <button
            class="c-item item-btn"
            onclick={() => (expanded = hasExpanded ? null : `proj:${g.projectDir}`)}
            title="Click to view project dependencies"
          >
            <div class="item-text">
              <span class="title">{g.projectName}</span>
              <span class="sub path mono">{shortPath(g.projectDir, home)} · {g.items.length} dep{g.items.length === 1 ? "" : "s"}</span>
            </div>
          </button>

          <div class="c-origin">
            <div class="chip-row">
              {#each g.items as it}
                <span class="quiet-chip mono">{it.name.split("/").pop()}</span>
              {/each}
            </div>
          </div>

          <div class="c-status">
            <div class="status-cell">
              {#if g.gitIdleDays !== null}
                <span class="dot {g.gitIdleDays >= 14 ? 'dot-dangling' : 'dot-unused'}"></span>
                <span class="status-label mono dim">{g.gitIdleDays}d</span>
              {:else}
                <span class="status-label mono dim">—</span>
              {/if}
            </div>
          </div>

          <div class="c-risk">
            <span class="risk-badge {g.items.some((i) => i.risk === 'caution') ? 'risk-caution' : 'risk-safe'}">
              {g.items.some((i) => i.risk === 'caution') ? 'caution' : 'safe'}
            </span>
          </div>

          <div class="c-size mono">
            <div class="size-main">
              <span class="num">{num}</span>
              <span class="unit">{unit}</span>
            </div>
          </div>
        </div>

        {#if hasExpanded}
          <div class="inspector-drawer">
            <div class="inspector-grid">
              <div class="inspector-col">
                <div class="col-header">
                  <span class="col-title">Project Details</span>
                </div>
                <div class="spec-row">
                  <span class="spec-label">Project Root</span>
                  <div class="spec-val">
                    <div class="copy-field">
                      <span class="mono truncated-id">{g.projectDir}</span>
                      <button
                        type="button"
                        class="copy-btn"
                        onclick={() => copyText(g.projectDir, `pdir-${g.projectDir}`)}
                        title="Copy path"
                      >
                        {copiedId === `pdir-${g.projectDir}` ? "Copied" : "Copy"}
                      </button>
                    </div>
                  </div>
                </div>
                <div class="spec-row">
                  <span class="spec-label">Git Activity</span>
                  <span class="spec-val mono dim">
                    {g.gitIdleDays !== null ? `Last commit / checkout was ${g.gitIdleDays} days ago` : "No .git logs found"}
                  </span>
                </div>
                <div class="inspector-actions">
                  <button type="button" class="btn" onclick={() => api.reveal(g.projectDir)}>
                    Reveal in Finder
                  </button>
                  <button
                    type="button"
                    class="action-btn {g.allSelected ? 'active' : ''}"
                    onclick={() => toggleProject(g)}
                  >
                    {g.allSelected ? "Deselect All Deps" : "Select All Deps"} ({g.items.length})
                  </button>
                </div>
              </div>

              <div class="inspector-col">
                <div class="col-header">
                  <span class="col-title">Regenerable Dependencies ({fmtBytes(g.totalBytes)})</span>
                </div>
                <div class="project-deps-list">
                  {#each g.items as it}
                    <div class="dep-item-row">
                      <input
                        type="checkbox"
                        checked={selected.has(it.id)}
                        disabled={it.risk === 'advisory'}
                        onchange={() => onToggle(it.id)}
                      />
                      <span class="mono dep-name">{it.name}</span>
                      <span class="mono dep-size dim">{fmtBytes(it.bytes)}</span>
                      {#if it.restore}
                        <button
                          type="button"
                          class="copy-btn"
                          onclick={() => copyText(it.restore!, `dep-rst-${it.id}`)}
                          title="Copy restore command"
                        >
                          {copiedId === `dep-rst-${it.id}` ? "✓ Copied" : "Restore Cmd"}
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
              </div>
            </div>
          </div>
        {/if}
      {/each}
    {:else}
      {#if topSpacer > 0}
        <div style="height: {topSpacer}px; flex: none;"></div>
      {/if}
      {#each visibleFindings as f, i (f.id)}
      {@const globalIdx = isWindowed ? startIndex + i : i}
      {@const [num, unit] = bytesParts(f.bytes)}
      {@const advisory = f.risk === "advisory"}
      {@const dockerType = getDockerType(f)}
      {@const meta = (f.meta ?? {}) as Record<string, any>}
      {@const origin = getOriginInfo(f, dockerType, meta)}
      {@const status = getStatusInfo(f, dockerType, meta)}
      {@const resourceId = getResourceId(f, dockerType, meta)}
      {@const cmd = getCommandSnippet(f, dockerType, meta)}

      <div
        class="row item"
        class:on={selected.has(f.id)}
        class:focused={focusedIndex === globalIdx}
        class:is-expanded={expanded === f.id}
        role="presentation"
      >
        <div class="c-check">
          <input
            type="checkbox"
            checked={selected.has(f.id)}
            disabled={advisory}
            onchange={() => onToggle(f.id)}
            aria-label={`Select ${f.name}`}
            title={advisory ? "Advisory item — protected from deletion" : `Select ${f.name}`}
          />
        </div>

        <button
          class="c-item item-btn"
          onclick={() => (expanded = expanded === f.id ? null : f.id)}
          title="Click to inspect technical details"
        >
          <div class="item-text">
            <span class="title">
              {#if dockerType === "container"}
                {meta.name || f.name}
              {:else if dockerType === "volume"}
                {meta.volume_name || f.name}
              {:else if dockerType === "image"}
                {meta.tag || f.name}
              {:else if dockerType === "build_cache"}
                Docker BuildKit Cache
              {:else}
                {f.name}
              {/if}
            </span>
            <span class="sub">
              {#if dockerType === "container"}
                {meta.image || "unknown"}{meta.volumes && meta.volumes.length > 0 ? ` · ${meta.volumes.length} vol` : " · ephemeral"}
              {:else if dockerType === "volume"}
                {meta.mountpoint ? shortPath(meta.mountpoint, home) : "unattached volume"}
              {:else if dockerType === "image"}
                {meta.all_tags && meta.all_tags.length > 1 ? `${meta.all_tags.length} tags · ` : ""}{f.files} layers
              {:else if dockerType === "build_cache"}
                {meta.total_layers ?? f.files} layers ({meta.reclaimable_layers ?? 0} reclaimable)
              {:else}
                {f.detail}
                {#if f.path}
                  <span class="path mono">· {shortPath(f.path, home)}</span>
                {/if}
              {/if}
            </span>
          </div>
        </button>

        <div class="c-origin">
          <span class="origin-tag {origin.cls ?? ''}">{origin.label}</span>
        </div>

        <div class="c-status">
          <div class="status-cell">
            {#if status.dot}
              <span class="dot dot-{status.dot}"></span>
            {/if}
            <span class="status-label {status.dot ?? ''} {status.isIdle ? 'mono dim' : ''}">
              {status.label}
            </span>
          </div>
        </div>

        <div class="c-risk">
          <button
            type="button"
            class="risk-badge risk-{f.risk} hasTip"
            onmouseenter={(e) => showTip(e, f)}
            onmouseleave={() => (tip = null)}
            onfocus={(e) => showTip(e, f)}
            onblur={() => (tip = null)}>{f.risk}</button
          >
        </div>

        <div class="c-size mono">
          <div class="size-main">
            <span class="num">{num}</span>
            <span class="unit">{unit}</span>
          </div>
          {#if f.shared_bytes > 0}
            <span
              class="size-shared"
              title="Shared via hardlinks, image layers or APFS clones. Freed only when last reference is removed."
            >
              +{fmtBytes(f.shared_bytes)} shared
            </span>
          {/if}
        </div>
      </div>

      <!-- Developer Inspector Drawer (Clean 2-Column Standard) -->
      {#if expanded === f.id}
        <div class="inspector-drawer">
          <div class="inspector-grid">
            <!-- Left Column: Technical Overview & Identifiers -->
            <div class="inspector-col">
              <div class="col-header">
                <span class="col-title">Resource Specification</span>
              </div>

              <!-- Resource ID with One-Click Copy -->
              <div class="spec-row">
                <span class="spec-label">
                  {meta.udid
                    ? "UDID"
                    : dockerType === "container"
                    ? "Container ID"
                    : dockerType === "volume"
                    ? "Volume Name"
                    : dockerType === "image"
                    ? "Image ID"
                    : "Identifier"}
                </span>
                <div class="spec-val">
                  <div class="copy-field">
                    <span class="mono truncated-id">{resourceId}</span>
                    <button
                      type="button"
                      class="copy-btn"
                      onclick={() => copyText(resourceId, `id-${f.id}`)}
                      title="Copy to clipboard"
                    >
                      {copiedId === `id-${f.id}` ? "✓ Copied" : "Copy"}
                    </button>
                  </div>
                </div>
              </div>

              {#if meta.udid}
                <div class="spec-row">
                  <span class="spec-label">Runtime</span>
                  <span class="spec-val mono dim">{meta.runtime_name || meta.runtime || "unknown"}</span>
                </div>
                <div class="spec-row">
                  <span class="spec-label">Device Type</span>
                  <span class="spec-val mono dim">{meta.device_type || "unknown"}</span>
                </div>
                <div class="spec-row">
                  <span class="spec-label">State</span>
                  <span class="spec-val mono dim">{meta.state || "Shutdown"}</span>
                </div>
              {/if}

              {#if dockerType === "image" && meta.all_tags && meta.all_tags.length > 1}
                <div class="spec-row">
                  <span class="spec-label">All Tags</span>
                  <div class="spec-val chip-row">
                    {#each meta.all_tags as tag}
                      <span class="quiet-chip mono">{tag}</span>
                    {/each}
                  </div>
                </div>
              {/if}

              {#if dockerType === "container"}
                <div class="spec-row">
                  <span class="spec-label">Image</span>
                  <span class="spec-val mono dim">{meta.image || "unknown"}</span>
                </div>
                <div class="spec-row">
                  <span class="spec-label">Attached Volumes</span>
                  <div class="spec-val">
                    {#if meta.volumes && meta.volumes.length > 0}
                      <div class="chip-row">
                        {#each meta.volumes as v}
                          <span class="quiet-chip mono">{v}</span>
                        {/each}
                      </div>
                    {:else}
                      <span class="dim faint">None (ephemeral storage only)</span>
                    {/if}
                  </div>
                </div>
              {/if}

              {#if dockerType === "volume"}
                <div class="spec-row">
                  <span class="spec-label">Driver</span>
                  <span class="spec-val mono dim">{meta.driver || "local"}</span>
                </div>
                {#if meta.mountpoint}
                  <div class="spec-row">
                    <span class="spec-label">Mountpoint</span>
                    <span class="spec-val mono dim">{meta.mountpoint}</span>
                  </div>
                {/if}
                <div class="spec-row">
                  <span class="spec-label">Containers</span>
                  <div class="spec-val">
                    {#if meta.containers && meta.containers.length > 0}
                      <div class="chip-row">
                        {#each meta.containers as c}
                          <span class="quiet-chip mono">{c}</span>
                        {/each}
                      </div>
                    {:else}
                      <span class="dim caution">Dangling (not attached to any container)</span>
                    {/if}
                  </div>
                </div>
              {/if}

              {#if !dockerType && f.path}
                <div class="spec-row">
                  <span class="spec-label">Path</span>
                  <span class="spec-val mono dim">{f.path}</span>
                </div>
                <div class="spec-row">
                  <span class="spec-label">Files</span>
                  <span class="spec-val mono dim">{f.files.toLocaleString()} entries</span>
                </div>
              {/if}

              <!-- Disk Allocation Breakdown -->
              <div class="spec-row">
                <span class="spec-label">Allocation</span>
                <div class="spec-val mono footprint-val">
                  <span class="footprint-item">
                    <span class="dim">Exclusive:</span>
                    <strong class="safe">{fmtBytes(f.bytes)}</strong>
                  </span>
                  {#if f.shared_bytes > 0}
                    <span class="footprint-item">
                      <span class="dim">Shared:</span>
                      <strong class="amber">{fmtBytes(f.shared_bytes)}</strong>
                    </span>
                  {/if}
                  {#if meta.size_root_fs}
                    <span class="footprint-item">
                      <span class="dim">RootFS:</span>
                      <span>{fmtBytes(meta.size_root_fs)}</span>
                    </span>
                  {/if}
                </div>
              </div>
            </div>

            <!-- Right Column: Safety Policy & Terminal Command -->
            <div class="inspector-col">
              <div class="col-header">
                <span class="col-title">Safety & Operations</span>
              </div>

              <!-- Policy Box -->
              <div class="policy-card policy-{f.risk}">
                <span class="policy-title">
                  {#if f.risk === "safe"}
                    Safe to Reclaim
                  {:else if f.risk === "caution"}
                    Caution / Recoverable
                  {:else}
                    Protected / Advisory
                  {/if}
                </span>
                <p class="policy-body">
                  {getSafetyPolicyText(f, dockerType, meta)}
                </p>
              </div>

              <!-- Terminal Command Snippet Box -->
              {#if cmd}
                <div class="command-box">
                  <div class="command-header">
                    <span class="command-title mono">{cmd.label}</span>
                    <button
                      type="button"
                      class="copy-btn copy-cmd-btn"
                      onclick={() => copyText(cmd.snippet, `cmd-${f.id}`)}
                      title="Copy command to clipboard"
                    >
                      {copiedId === `cmd-${f.id}` ? "✓ Copied" : "Copy Command"}
                    </button>
                  </div>
                  <pre class="command-pre mono"><code>$ {cmd.snippet}</code></pre>
                </div>
              {/if}

              <!-- Native File Actions -->
              {#if f.path}
                <div class="inspector-actions">
                  <button type="button" class="btn" onclick={() => api.reveal(f.path!)}>
                    Reveal in Finder
                  </button>
                  <button type="button" class="btn" onclick={() => api.quickLook(f.path!)}>
                    Quick Look
                  </button>
                </div>
              {/if}
            </div>
          </div>
        </div>
      {/if}
    {/each}
    {#if bottomSpacer > 0}
      <div style="height: {bottomSpacer}px; flex: none;"></div>
    {/if}
    {/if}

    {#if filteredFindings.length === 0}
      <div class="empty">
        {#if searchQuery}
          No items match "{searchQuery}". Press Esc to clear filter.
        {:else if isDocker && dockerTab !== "all"}
          No {dockerTab} found in this category.
        {:else}
          Nothing here. Either it is already clean, or try a deep sweep.
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if tip}
  <div class="tip tip-{tip.risk}" style="left:{tip.x}px; top:{tip.y}px" role="tooltip">
    <strong>{tip.meaning}</strong>
    <span>{tip.reason}</span>
  </div>
{/if}

<style>
  .hasTip { cursor: help; }
  .tip {
    position: fixed;
    z-index: 50;
    width: 340px;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--ink-dim);
    background: var(--bg-surface-elevated);
    border: 1px solid var(--line-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-popup);
    pointer-events: none;
  }
  .tip strong { color: var(--ink); font-weight: 600; font-size: 13px; }
  .tip-safe strong { color: var(--mint-bright); }
  .tip-caution strong { color: var(--amber); }
  .tip-advisory strong { color: var(--blue); }

  .table { display: flex; flex-direction: column; height: 100%; min-height: 0; }

  /* Unified Command & Segmented Navigation Bar */
  .unified-bar {
    flex: none;
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 0 18px;
    border-bottom: 1px solid var(--line);
    background: var(--bg-surface);
  }

  .bar-left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }

  .segmented-control {
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    padding: 3px;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex: none;
  }
  .segment {
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--ink-dim);
    font-size: 12.5px;
    font-weight: 500;
    padding: 4px 11px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    transition: all 120ms ease;
    white-space: nowrap;
  }
  .segment:hover {
    color: var(--ink);
  }
  .segment.active {
    background: var(--bg-surface-elevated);
    color: var(--ink);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.08);
  }
  .segment-badge {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    color: var(--ink-faint);
  }
  .segment.active .segment-badge {
    color: var(--ink);
    border-color: rgba(255, 255, 255, 0.16);
  }

  .sub-segmented-control {
    background: var(--bg-canvas);
    border: 1px solid var(--line-soft);
    border-radius: var(--radius-sm);
    padding: 2px;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }
  .sub-segment {
    background: transparent;
    border: none;
    border-radius: var(--radius-xs);
    color: var(--ink-faint);
    font-size: 11.5px;
    padding: 3px 8px;
    cursor: pointer;
    transition: all 100ms ease;
    white-space: nowrap;
  }
  .sub-segment:hover {
    color: var(--ink);
  }
  .sub-segment.active {
    background: var(--bg-surface-elevated);
    color: var(--ink);
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    border-radius: var(--radius-sm);
    padding: 4px 10px;
    height: 30px;
    width: 200px;
    transition: border-color 120ms ease, width 140ms ease;
    flex: none;
  }
  .search-box:focus-within {
    border-color: var(--mint-bright);
    width: 240px;
  }
  .search-glyph {
    color: var(--ink-faint);
    font-size: 14px;
    line-height: 1;
    pointer-events: none;
  }
  .search-input {
    background: transparent;
    border: none;
    outline: none;
    color: var(--ink);
    font-size: 12.5px;
    width: 100%;
    padding: 0;
  }
  .search-input::placeholder {
    color: var(--ink-faint);
  }
  .clear-btn {
    background: none;
    border: none;
    color: var(--ink-faint);
    cursor: pointer;
    font-size: 11px;
    padding: 0;
  }
  .clear-btn:hover {
    color: var(--ink);
  }

  .bar-actions {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: none;
  }
  .action-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-surface-elevated);
    border: 1px solid var(--line);
    border-radius: var(--radius-sm);
    color: var(--ink-dim);
    font-size: 12px;
    font-weight: 500;
    padding: 5px 12px;
    cursor: pointer;
    transition: all 120ms ease;
    white-space: nowrap;
  }
  .action-btn:hover {
    border-color: rgba(255, 255, 255, 0.2);
    color: var(--ink);
    background: var(--bg-surface-hover);
  }
  .action-btn.active {
    background: rgba(16, 185, 129, 0.12);
    border-color: rgba(16, 185, 129, 0.4);
    color: var(--mint-bright);
  }
  .item-count {
    font-size: 12px;
    white-space: nowrap;
  }

  /* Strict Column Alignment */
  .row {
    display: grid;
    grid-template-columns: 36px minmax(220px, 1fr) 120px 100px 85px 125px;
    align-items: center;
    gap: 12px;
    padding: 0 18px;
  }

  .head {
    flex: none;
    height: 34px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--ink-faint);
    border-bottom: 1px solid var(--line);
    background: var(--bg-surface);
  }

  .col-hdr {
    display: flex;
    align-items: center;
    height: 100%;
  }

  .col-btn {
    background: none;
    border: none;
    padding: 0;
    font-size: inherit;
    font-weight: inherit;
    text-transform: inherit;
    letter-spacing: inherit;
    color: inherit;
    display: flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
    text-align: left;
    height: 100%;
    transition: color 100ms ease;
  }
  .col-btn:hover {
    color: var(--ink);
  }
  .sort-right {
    justify-content: flex-end;
  }
  .sort-icon {
    font-size: 9px;
    color: var(--mint-bright);
  }

  .body { flex: 1; min-height: 0; overflow-y: auto; }

  .item {
    min-height: 56px;
    border-bottom: 1px solid var(--line-soft);
    border-left: 3px solid transparent;
    transition: background 120ms ease;
  }
  .item:hover { background: rgba(255, 255, 255, 0.03); }
  .item.on { background: rgba(16, 185, 129, 0.04); }
  .item.focused {
    border-left-color: var(--mint-bright);
    background: rgba(255, 255, 255, 0.045);
  }
  .item.is-expanded {
    background: rgba(255, 255, 255, 0.025);
    border-bottom-color: var(--line);
  }

  .c-check {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .item-btn {
    background: none;
    border: none;
    padding: 8px 0;
    text-align: left;
    min-width: 0;
    cursor: pointer;
  }
  .item-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    font-size: 14.5px;
    font-weight: 550;
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    letter-spacing: -0.01em;
  }
  .sub {
    font-size: 12.5px;
    color: var(--ink-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path { color: var(--ink-faint); font-size: 12px; }

  /* Origin Column */
  .c-origin {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .origin-tag {
    font-size: 12px;
    color: var(--ink-dim);
  }
  .origin-registry { color: #38bdf8; }
  .origin-local { color: #c084fc; }
  .origin-untagged { color: var(--ink-faint); }
  .origin-sim { color: #a78bfa; }

  /* Status Micro-Indicators (7px Dots) */
  .c-status {
    overflow: hidden;
    white-space: nowrap;
  }
  .status-cell {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
  }
  .dot-active {
    background: var(--blue);
    box-shadow: 0 0 6px rgba(56, 189, 248, 0.6);
  }
  .dot-stopped {
    background: var(--ink-faint);
  }
  .dot-dangling {
    background: var(--amber);
  }
  .dot-unused {
    background: var(--mint-bright);
  }
  .status-label {
    color: var(--ink-dim);
  }
  .status-label.active { color: var(--blue); }
  .status-label.dangling { color: var(--amber); }

  /* Risk Badges */
  .c-risk {
    display: flex;
    align-items: center;
  }
  .risk-badge {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: var(--radius-xs);
    border: 1px solid transparent;
  }
  .risk-safe {
    color: var(--mint-bright);
    background: rgba(16, 185, 129, 0.1);
    border-color: rgba(16, 185, 129, 0.25);
  }
  .risk-caution {
    color: var(--amber);
    background: rgba(251, 191, 36, 0.1);
    border-color: rgba(251, 191, 36, 0.25);
  }
  .risk-advisory {
    color: var(--blue);
    background: rgba(56, 189, 248, 0.1);
    border-color: rgba(56, 189, 248, 0.25);
  }

  /* Size Column */
  .c-size {
    text-align: right;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
  }
  .size-main {
    display: flex;
    align-items: baseline;
  }
  .num { font-size: 15.5px; font-weight: 650; color: var(--ink); }
  .unit { font-size: 11.5px; color: var(--ink-dim); margin-left: 3px; font-weight: 500; }
  .size-shared { font-size: 11px; color: var(--amber); opacity: 0.9; }

  /* Developer Inspector Drawer (2-Column Standard) */
  .inspector-drawer {
    padding: 18px 24px 22px 52px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--line);
    user-select: text;
  }
  .inspector-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 28px;
  }
  .inspector-col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .col-header {
    border-bottom: 1px solid var(--line-soft);
    padding-bottom: 5px;
    margin-bottom: 3px;
  }
  .col-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--ink-faint);
  }
  .spec-row {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr);
    gap: 12px;
    font-size: 12.5px;
    align-items: baseline;
  }
  .spec-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-faint);
  }
  .spec-val {
    color: var(--ink-dim);
    min-width: 0;
    word-break: break-all;
  }
  .copy-field {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    border-radius: var(--radius-xs);
    padding: 3px 8px;
    max-width: 100%;
  }
  .truncated-id {
    font-size: 12px;
    color: var(--ink);
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .copy-btn {
    font-size: 11.5px;
    padding: 2px 7px;
    border-radius: var(--radius-xs);
    background: var(--bg-surface-elevated);
    border: 1px solid var(--line);
    color: var(--ink-dim);
    cursor: pointer;
    transition: all 100ms ease;
    white-space: nowrap;
  }
  .copy-btn:hover {
    background: var(--bg-surface-hover);
    color: var(--ink);
    border-color: rgba(255, 255, 255, 0.2);
  }
  .copy-cmd-btn {
    padding: 3px 10px;
    font-size: 11.5px;
  }
  .chip-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .quiet-chip {
    font-size: 11.5px;
    padding: 2px 8px;
    border-radius: var(--radius-xs);
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    color: var(--ink-dim);
  }
  .footprint-val {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    font-size: 12px;
  }
  .footprint-item {
    display: flex;
    gap: 5px;
  }
  .safe { color: var(--mint-bright); }
  .amber { color: var(--amber); }

  .policy-card {
    padding: 10px 14px;
    border-radius: var(--radius-md);
    border: 1px solid var(--line);
    background: var(--bg-canvas);
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .policy-title {
    font-size: 12px;
    font-weight: 600;
  }
  .policy-body {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--ink-dim);
  }
  .policy-safe .policy-title { color: var(--mint-bright); }
  .policy-caution .policy-title { color: var(--amber); }
  .policy-advisory .policy-title { color: var(--blue); }

  .command-box {
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .command-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line-soft);
    background: rgba(255, 255, 255, 0.02);
  }
  .command-title {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-faint);
  }
  .command-pre {
    margin: 0;
    padding: 10px 12px;
    font-size: 12.5px;
    color: var(--ink);
    overflow-x: auto;
  }
  .command-pre code {
    font-family: var(--mono);
  }

  .inspector-actions {
    display: flex;
    gap: 10px;
    margin-top: 4px;
  }

  .project-deps-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .dep-item-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12.5px;
    padding: 6px 10px;
    background: var(--bg-canvas);
    border: 1px solid var(--line-soft);
    border-radius: var(--radius-sm);
  }
  .dep-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--ink);
  }
  .dep-size {
    color: var(--mint-bright);
    font-weight: 500;
  }

  .empty {
    padding: 60px 24px;
    text-align: center;
    color: var(--ink-faint);
    font-size: 14px;
  }
</style>
