<script lang="ts">
  import type { TaskStatus } from '$lib/api/index.js';
  import { taskStatusLabel, statusBgClass } from './task-utils.js';

  interface Props {
    status: TaskStatus | string;
  }

  let { status }: Props = $props();
</script>

<span
  class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium {statusBgClass(
    status
  )}"
>
  {taskStatusLabel(status)}
</span>
