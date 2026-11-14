// Times the benchmark the way `marionette bench` reports it (decision 0284): untimed warm-up
// frames, then timed frames; writes medians and percentiles as JSON and quits. Command line:
//   -benchOut <file.json>   where to write the report (default: next to the player)
//   -benchFrames <n>        timed frames (default 1000)
//   -benchWarmup <n>        untimed frames first (default 120)
// GPU time needs Player Settings > Frame Timing Stats on (the builder turns it on).
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using UnityEngine;

namespace MarionetteBench
{
    public class RealisticBenchTimer : MonoBehaviour
    {
        int warmup = 120, frames = 1000, seen;
        string outPath;
        readonly List<double> frameMs = new List<double>();
        readonly List<double> cpuMainMs = new List<double>();
        readonly List<double> cpuRenderMs = new List<double>();
        readonly List<double> gpuMs = new List<double>();
        readonly FrameTiming[] timing = new FrameTiming[1];
        double startedAt;

        void Start()
        {
            QualitySettings.vSyncCount = 0;
            Application.targetFrameRate = -1;
            string[] args = Environment.GetCommandLineArgs();
            for (int i = 0; i + 1 < args.Length; i++)
            {
                if (args[i] == "-benchOut") outPath = args[i + 1];
                if (args[i] == "-benchFrames") int.TryParse(args[i + 1], out frames);
                if (args[i] == "-benchWarmup") int.TryParse(args[i + 1], out warmup);
            }
            if (string.IsNullOrEmpty(outPath))
                outPath = Path.Combine(Path.GetDirectoryName(Application.dataPath) ?? ".", "realistic-bench.json");
            startedAt = Time.realtimeSinceStartupAsDouble;
        }

        void Update()
        {
            FrameTimingManager.CaptureFrameTimings();
            seen++;
            if (seen <= warmup) return;
            frameMs.Add(Time.unscaledDeltaTime * 1000.0);
            if (FrameTimingManager.GetLatestTimings(1, timing) > 0)
            {
                cpuMainMs.Add(timing[0].cpuMainThreadFrameTime);
                cpuRenderMs.Add(timing[0].cpuRenderThreadFrameTime);
                gpuMs.Add(timing[0].gpuFrameTime);
            }
            if (frameMs.Count >= frames) Finish();
        }

        static string Stats(List<double> xs)
        {
            if (xs.Count == 0) return "null";
            var s = new List<double>(xs);
            s.Sort();
            double sum = 0;
            foreach (var x in s) sum += x;
            Func<double, double> p = q => s[Math.Min(s.Count - 1, (int)Math.Floor(q * (s.Count - 1) + 0.5))];
            return string.Format(CultureInfo.InvariantCulture,
                "{{\"count\": {0}, \"avg_ms\": {1:0.###}, \"p50_ms\": {2:0.###}, \"p99_ms\": {3:0.###}, \"max_ms\": {4:0.###}}}",
                s.Count, sum / s.Count, p(0.5), p(0.99), s[s.Count - 1]);
        }

        void Finish()
        {
            var sb = new StringBuilder();
            sb.Append("{\n");
            sb.AppendFormat("  \"engine\": \"Unity {0}\",\n", Application.unityVersion);
            sb.AppendFormat("  \"gpu_name\": \"{0}\",\n", SystemInfo.graphicsDeviceName.Replace("\"", "'"));
            sb.AppendFormat("  \"api\": \"{0}\",\n", SystemInfo.graphicsDeviceType);
            sb.AppendFormat("  \"size\": \"{0}x{1}\",\n", Screen.width, Screen.height);
            sb.AppendFormat(CultureInfo.InvariantCulture, "  \"load_s\": {0:0.###},\n", startedAt);
            sb.AppendFormat("  \"warmup\": {0},\n", warmup);
            sb.AppendFormat("  \"frame\": {0},\n", Stats(frameMs));
            sb.AppendFormat("  \"cpu_main\": {0},\n", Stats(cpuMainMs));
            sb.AppendFormat("  \"cpu_render\": {0},\n", Stats(cpuRenderMs));
            sb.AppendFormat("  \"gpu\": {0}\n", Stats(gpuMs));
            sb.Append("}\n");
            File.WriteAllText(outPath, sb.ToString());
            Application.Quit();
        }
    }
}
