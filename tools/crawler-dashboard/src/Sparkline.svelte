<script lang="ts">
  let {values,color="#326d50",label}:{values:number[];color?:string;label:string}=$props();
  const finite=$derived(values.filter(Number.isFinite));
  const max=$derived(Math.max(1,...finite));
  const points=$derived(finite.map((v,i)=>`${i/Math.max(1,finite.length-1)*240},${42-v/max*38}`).join(' '));
</script>
<svg viewBox="0 0 240 46" role="img" aria-label={label} style:color={color} preserveAspectRatio="none"><line x1="0" x2="240" y1="43" y2="43"/>{#if finite.length>1}<polyline {points}/>{/if}</svg>
<style>svg{display:block;width:100%;height:46px}line{stroke:#dce4de}polyline{fill:none;stroke:currentColor;stroke-width:2;vector-effect:non-scaling-stroke}</style>
