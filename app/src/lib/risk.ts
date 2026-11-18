import type { Finding, Risk } from "./api";

/** What each label means in general. Shown as the tooltip's headline. */
export const RISK_MEANING: Record<Risk, string> = {
  safe: "Safe — regenerates by itself. Nothing here is yours; the tool that made it rebuilds it.",
  caution: "Caution — recoverable, but it costs you something: time, bandwidth, or a manual step.",
  advisory: "Advisory — reported for your information. Woosh will never delete this.",
};

/** Per-finding reasons that don't follow from the domain alone. Keyed by finding id. */
const BY_ID: Record<string, string> = {
  "cache:pnpm":
    "pnpm hardlinks every project's node_modules to this store. Deleting it forces each project to re-download, and projects linked to it can break until reinstalled.",
  "cache:go_mod": "Go re-downloads modules on the next build, which needs network access and may fail offline.",
  "cache:maven": "Maven re-downloads every dependency on the next build. Locally installed artifacts that were never published are lost for good.",
  "cache:xcode_archives": "Archives hold your past release builds and dSYMs. Without them old crash reports can't be symbolicated, and they can't be regenerated.",
  "cache:simulator_runtimes": "Each runtime is a 3–8 GB download. It is recoverable, but slow to fetch again.",
  "cache:android": "Virtual devices keep their own state (installed apps, snapshots). Deleting an AVD resets it entirely.",
  "cache:trash": "Items in the Trash are already deleted from your view. Emptying it is permanent — it can't be undone.",
  "docker:vm_disk":
    "A sparse VM image owned by Docker Desktop. It can only shrink from inside Docker, so start Docker and prune there.",
  "docker:build_cache": "Intermediate image layers. Docker rebuilds them on the next build, just more slowly.",
  "docker:images": "Unreferenced images. Anything you still need must be pulled or rebuilt, which takes time and network.",
  "docker:containers": "Stopped containers keep a writable layer. Anything written inside one and not in a volume is lost.",
  "docker:volumes": "Dangling volumes can hold databases and other state. Their data is not recoverable once removed.",
  "ollama:blobs": "Raw model files. Large downloads, and a model you've since removed upstream may not be available again.",
};

/** Fallbacks by domain, then by risk. */
const BY_DOMAIN: Partial<Record<Finding["domain"], Partial<Record<Risk, string>>>> = {
  venv: {
    caution:
      "Anything importing from this environment stops working until it's recreated. Woosh writes req-inplace.txt into the project first, so a reinstall restores it.",
  },
  node_modules: {
    caution:
      "The project won't run until you reinstall, which needs network access. Any hand-edited files inside node_modules are lost.",
  },
  global_python: {
    caution:
      "Scripts and tools that rely on these globally installed packages can stop working. Essential packages are kept.",
    advisory: "Apple's system Python is protected by SIP and can't be modified, so it's shown for reference only.",
  },
  ai_model: {
    caution:
      "Model weights are large and slow to download again. Fine-tuned or locally converted models may not be re-obtainable.",
  },
  artifact: { safe: "Build output (target/, dist/, build/…). Your build command regenerates it from source." },
  cache: { safe: "A download or build cache. The tool refetches what it needs on next use." },
  large_file: {
    caution:
      "Your file. Woosh moves it to the Finder Trash; Put Back restores it to its original path.",
    advisory:
      "A media or app package bundle (.photoslibrary, .app, etc.). Manage or delete it inside its owning app.",
  },
};

export function explainRisk(f: Finding): { meaning: string; reason: string } {
  if (f.domain === "docker") {
    const meta = f.meta as Record<string, unknown> | undefined;
    if (meta?.docker_type === "container") {
      if (meta.state === "running") {
        return {
          meaning: RISK_MEANING.advisory,
          reason: "Active running container. Woosh does not delete running containers to prevent data loss. Stop it first.",
        };
      }
      return {
        meaning: RISK_MEANING.caution,
        reason: "Stopped container writable layer. Deleting frees container changes; named volumes are preserved.",
      };
    }
    if (meta?.docker_type === "volume") {
      if (meta.is_active) {
        return {
          meaning: RISK_MEANING.advisory,
          reason: "Attached volume in active use by running container(s). Protected from deletion.",
        };
      }
      return {
        meaning: RISK_MEANING.caution,
        reason: "Dangling volume. Not mounted to any running container. Deleting permanently removes its stored data.",
      };
    }
    if (meta?.docker_type === "image") {
      if (f.risk === "advisory") {
        return {
          meaning: RISK_MEANING.advisory,
          reason: "Image is actively in use by existing containers.",
        };
      }
      if (meta.is_pullable) {
        return {
          meaning: RISK_MEANING.caution,
          reason: `Pullable registry image. Safe to remove — re-download anytime with docker pull ${meta.tag ?? ""}.`,
        };
      }
      if (meta.origin === "local_build") {
        return {
          meaning: RISK_MEANING.caution,
          reason: "Built on device. Cannot be re-pulled from registry; must be recompiled/rebuilt from source code.",
        };
      }
      return {
        meaning: RISK_MEANING.caution,
        reason: "Untagged build layer from previous local builds. Safe to reclaim.",
      };
    }
  }

  const reason =
    BY_ID[f.id] ??
    BY_DOMAIN[f.domain]?.[f.risk] ??
    (f.risk === "safe"
      ? "Regenerated automatically the next time it's needed."
      : f.risk === "caution"
        ? "Recoverable, but not free — see the restore step in the details."
        : "Reported only.");
  return { meaning: RISK_MEANING[f.risk], reason };
}
