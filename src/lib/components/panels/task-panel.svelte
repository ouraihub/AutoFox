<script lang="ts">
  import { Button, Empty } from '$lib/components/ui';
  import type { TaskSummary } from '$lib/ipc/client';

  type Props = {
    tasks: TaskSummary[];
    onRefresh: () => void;
    onCancel: (taskId: string) => void;
  };

  const { tasks, onRefresh, onCancel }: Props = $props();
</script>

<!-- `.panel-heading`, `.rows`, `.row` are global primitives in `src/app.css`
     (shared by every panel, so no single component's scoped <style> owns them). -->

<div class="panel-heading">
  <h2>任务</h2>
  <Button variant="outline" onclick={onRefresh}>刷新</Button>
</div>

{#if tasks.length === 0}
  <Empty title="还没有下载任务" description="解析链接后选择一个格式开始下载。" />
{:else}
  <div class="rows">
    {#each tasks as task (task.taskId)}
      <div class="row">
        <div>
          <strong>{task.kind}</strong>
          <small>{task.state}{task.outputPath ? ` · ${task.outputPath}` : ''}</small>
        </div>
        {#if task.state === 'downloading'}
          <!-- Cancelling takes the row's own id rather than "the running task":
               with up to four active downloads, the button next to a row has to
               stop that row. -->
          <Button variant="destructive" onclick={() => onCancel(task.taskId)}>取消</Button>
        {/if}
      </div>
    {/each}
  </div>
{/if}
