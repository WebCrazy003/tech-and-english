import { api, type DeepPartial, type Settings } from "../../lib/api";
import { useApp } from "../../stores/app";
import { toastError } from "../../stores/toast";

/** Save a settings change and update the store (the backend validates). */
export function useSettingsPatch() {
  return (patch: DeepPartial<Settings>) =>
    api
      .updateSettings(patch)
      .then((settings) => useApp.setState({ settings }))
      .catch(toastError);
}
