// Builds Marionette's realistic render benchmark (decision 0284) as a Unity scene and player,
// from the folder `marionette op perf_realistic_scene` writes. Needs URP (Forward+ is set).
//
// Batch mode (no window):
//   Unity -batchmode -quit -projectPath <project> -executeMethod MarionetteBench.RealisticBenchBuilder.Build
//         -benchFolder <generated folder> -benchPlayer <out>/RealisticBench.exe [-benchStatic]
// Or the menu: Marionette > Build realistic benchmark (asks for the folder).
//
// What it makes, under Assets/RealisticBench: the textures (BC7, mipmaps, 16x anisotropic),
// one mesh asset per model (one submesh per part), one URP Lit material per part, and a scene
// with the same camera, sun, cascaded shadows, static models and movers as the Marionette
// scene. `-benchStatic` marks the static models for Unity's static batching (off by default:
// Marionette has none, so the default compares like for like).
using System;
using System.Collections.Generic;
using System.IO;
using MarionetteBench;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;

namespace MarionetteBench
{
    public static class RealisticBenchBuilder
    {
        const string Root = "Assets/RealisticBench";

        [MenuItem("Marionette/Build realistic benchmark")]
        static void BuildFromMenu()
        {
            string folder = EditorUtility.OpenFolderPanel("Generated benchmark folder (has manifest.json)", "", "");
            if (string.IsNullOrEmpty(folder)) return;
            BuildScene(folder, false);
        }

        /// Entry point for -executeMethod.
        public static void Build()
        {
            try
            {
                string folder = Arg("-benchFolder");
                if (folder == null) throw new Exception("pass -benchFolder <folder with manifest.json>");
                bool staticBatching = Array.IndexOf(Environment.GetCommandLineArgs(), "-benchStatic") >= 0;
                string scene = BuildScene(folder, staticBatching);
                string player = Arg("-benchPlayer");
                if (player != null)
                {
                    var report = BuildPipeline.BuildPlayer(new BuildPlayerOptions
                    {
                        scenes = new[] { scene },
                        locationPathName = player,
                        target = BuildTarget.StandaloneWindows64,
                        options = BuildOptions.None,
                    });
                    if (report.summary.result != UnityEditor.Build.Reporting.BuildResult.Succeeded)
                        throw new Exception("player build failed: " + report.summary.result);
                }
                EditorApplication.Exit(0);
            }
            catch (Exception e)
            {
                Debug.LogError("realistic benchmark build failed: " + e);
                EditorApplication.Exit(1);
            }
        }

        static string Arg(string name)
        {
            string[] args = Environment.GetCommandLineArgs();
            for (int i = 0; i + 1 < args.Length; i++)
                if (args[i] == name) return args[i + 1];
            return null;
        }

        static string BuildScene(string folder, bool staticBatching)
        {
            var m = JsonUtility.FromJson<Manifest>(File.ReadAllText(Path.Combine(folder, "manifest.json")));
            if (m == null || m.format != "marionette.realistic-bench")
                throw new Exception("not a realistic benchmark manifest: " + folder);
            Directory.CreateDirectory(Root + "/Textures");
            Directory.CreateDirectory(Root + "/Meshes");
            Directory.CreateDirectory(Root + "/Materials");
            SetUpProject(m);

            // Textures: color (sRGB), normal map, metal-smoothness (linear), all BC7.
            AssetDatabase.StartAssetEditing();
            try
            {
                foreach (var model in m.models)
                    foreach (var rel in new[] { model.textures.color, model.textures.normal, model.textures.metal_smoothness })
                        File.Copy(Path.Combine(folder, rel), Root + "/Textures/" + Path.GetFileName(rel), true);
            }
            finally { AssetDatabase.StopAssetEditing(); }
            AssetDatabase.Refresh();
            AssetDatabase.StartAssetEditing();
            try
            {
                foreach (var model in m.models)
                {
                    Import(model.textures.color, m.texture_size, TextureImporterType.Default, true);
                    Import(model.textures.normal, m.texture_size, TextureImporterType.NormalMap, false);
                    Import(model.textures.metal_smoothness, m.texture_size, TextureImporterType.Default, false);
                }
            }
            finally { AssetDatabase.StopAssetEditing(); }

            // Meshes and materials.
            Shader lit = Shader.Find("Universal Render Pipeline/Lit");
            if (lit == null) throw new Exception("URP Lit shader not found: add the Universal RP package");
            var meshes = new Mesh[m.models.Length];
            var mats = new Material[m.models.Length][];
            for (int i = 0; i < m.models.Length; i++)
            {
                var model = m.models[i];
                meshes[i] = ReadMesh(Path.Combine(folder, model.mesh), model.name);
                AssetDatabase.CreateAsset(meshes[i], Root + "/Meshes/" + model.name + ".asset");
                var color = Tex(model.textures.color);
                var normal = Tex(model.textures.normal);
                var ms = Tex(model.textures.metal_smoothness);
                mats[i] = new Material[model.materials.Length];
                for (int k = 0; k < model.materials.Length; k++)
                {
                    var info = model.materials[k];
                    var mat = new Material(lit) { name = info.name, enableInstancing = true };
                    mat.SetTexture("_BaseMap", color);
                    // glTF factors are linear; material colors are entered in gamma space.
                    mat.SetColor("_BaseColor", new Color(info.tint[0], info.tint[1], info.tint[2], 1f).gamma);
                    mat.SetTexture("_BumpMap", normal);
                    mat.SetFloat("_BumpScale", 1f);
                    mat.EnableKeyword("_NORMALMAP");
                    // Metal from the map's R, smoothness from its A (factors are 1 in the manifest).
                    mat.SetTexture("_MetallicGlossMap", ms);
                    mat.SetFloat("_Smoothness", 1f);
                    mat.EnableKeyword("_METALLICSPECGLOSSMAP");
                    AssetDatabase.CreateAsset(mat, Root + "/Materials/" + info.name + ".mat");
                    mats[i][k] = mat;
                }
            }

            // Scene.
            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
            var cam = new GameObject("camera").AddComponent<Camera>();
            cam.transform.position = RealisticBenchAxes.Point(m.camera.position);
            cam.transform.LookAt(RealisticBenchAxes.Point(m.camera.target), Vector3.up);
            cam.fieldOfView = m.camera.fov_y_degrees;
            cam.nearClipPlane = m.camera.near;
            cam.farClipPlane = m.camera.far;
            cam.clearFlags = CameraClearFlags.SolidColor;
            cam.backgroundColor = new Color(m.camera.clear_color[0], m.camera.clear_color[1], m.camera.clear_color[2]).gamma;
            cam.allowMSAA = true;
            cam.allowHDR = true;
            cam.gameObject.tag = "MainCamera";

            var sun = new GameObject("sun").AddComponent<Light>();
            sun.type = LightType.Directional;
            sun.color = new Color(m.sun.color[0], m.sun.color[1], m.sun.color[2]).gamma;
            sun.intensity = m.sun.intensity;
            sun.shadows = LightShadows.Soft;
            sun.transform.rotation = Quaternion.LookRotation(RealisticBenchAxes.Point(m.sun.direction).normalized);

            RenderSettings.ambientMode = AmbientMode.Trilight;
            var sky = new Color(m.ambient.sky[0], m.ambient.sky[1], m.ambient.sky[2]) * 0.35f;
            var ground = new Color(m.ambient.ground[0], m.ambient.ground[1], m.ambient.ground[2]) * 0.35f;
            RenderSettings.ambientSkyColor = sky.gamma;
            RenderSettings.ambientEquatorColor = Color.Lerp(sky, ground, 0.5f).gamma;
            RenderSettings.ambientGroundColor = ground.gamma;
            RenderSettings.skybox = null;

            Place("ground", meshes[m.ground], mats[m.ground], Vector3.zero, Quaternion.identity, 1f, staticBatching);
            foreach (var inst in m.instances)
                Place(m.models[inst.model].name, meshes[inst.model], mats[inst.model],
                    RealisticBenchAxes.Point(inst.position), RealisticBenchAxes.Rotation(inst.rotation), inst.scale, staticBatching);

            var bench = new GameObject("bench");
            bench.AddComponent<RealisticBenchTimer>();
            var movers = bench.AddComponent<RealisticBenchMovers>();
            int n = m.movers.Length;
            movers.dt = m.dt;
            movers.movers = new Transform[n];
            movers.positions = new Vector3[n];
            movers.velocities = new Vector3[n];
            movers.gains = new float[n];
            movers.anchorOf = new int[n];
            movers.anchors = new Vector3[m.anchors.Length];
            for (int a = 0; a < m.anchors.Length; a++) movers.anchors[a] = RealisticBenchAxes.Point(m.anchors[a].position);
            for (int i = 0; i < n; i++)
            {
                var mv = m.movers[i];
                var p = RealisticBenchAxes.Point(mv.position);
                var go = Place(m.models[mv.model].name, meshes[mv.model], mats[mv.model], p,
                    RealisticBenchAxes.Rotation(mv.rotation), mv.scale, false);
                movers.movers[i] = go.transform;
                movers.positions[i] = p;
                movers.velocities[i] = RealisticBenchAxes.Point(mv.velocity);
                movers.gains[i] = mv.gain;
                movers.anchorOf[i] = mv.anchor;
            }

            string path = Root + "/RealisticBench.unity";
            EditorSceneManager.SaveScene(scene, path);
            EditorBuildSettings.scenes = new[] { new EditorBuildSettingsScene(path, true) };
            AssetDatabase.SaveAssets();
            Debug.Log(string.Format("realistic benchmark: {0} models, {1} static, {2} moving, {3} triangles",
                m.models.Length, m.instances.Length, n, m.triangles));
            return path;
        }

        static GameObject Place(string name, Mesh mesh, Material[] mats, Vector3 p, Quaternion q, float s, bool isStatic)
        {
            var go = new GameObject(name);
            go.AddComponent<MeshFilter>().sharedMesh = mesh;
            go.AddComponent<MeshRenderer>().sharedMaterials = mats;
            go.transform.SetPositionAndRotation(p, q);
            go.transform.localScale = new Vector3(s, s, s);
            if (isStatic) GameObjectUtility.SetStaticEditorFlags(go, StaticEditorFlags.BatchingStatic);
            return go;
        }

        static Texture2D Tex(string rel) =>
            AssetDatabase.LoadAssetAtPath<Texture2D>(Root + "/Textures/" + Path.GetFileName(rel));

        static void Import(string rel, int size, TextureImporterType type, bool srgb)
        {
            var ti = (TextureImporter)AssetImporter.GetAtPath(Root + "/Textures/" + Path.GetFileName(rel));
            ti.textureType = type;
            ti.sRGBTexture = srgb;
            ti.mipmapEnabled = true;
            ti.anisoLevel = 16;
            ti.filterMode = FilterMode.Trilinear;
            ti.wrapMode = TextureWrapMode.Repeat;
            ti.maxTextureSize = size;
            ti.textureCompression = TextureImporterCompression.CompressedHQ;
            var ps = ti.GetPlatformTextureSettings("Standalone");
            ps.overridden = true;
            ps.maxTextureSize = size;
            ps.format = TextureImporterFormat.BC7;
            ti.SetPlatformTextureSettings(ps);
            ti.SaveAndReimport();
        }

        /// Reads the `MBM1` mesh file: glTF axes and UVs, converted to Unity's (mirror X,
        /// reverse winding, flip V).
        static Mesh ReadMesh(string file, string name)
        {
            using (var r = new BinaryReader(File.OpenRead(file)))
            {
                if (new string(r.ReadChars(4)) != "MBM1") throw new Exception("not a mesh file: " + file);
                int parts = r.ReadInt32();
                var pos = new List<Vector3>();
                var nrm = new List<Vector3>();
                var uv = new List<Vector2>();
                var tris = new List<int[]>();
                var bases = new List<int>();
                for (int p = 0; p < parts; p++)
                {
                    int nv = r.ReadInt32(), ni = r.ReadInt32();
                    bases.Add(pos.Count);
                    for (int i = 0; i < nv; i++) pos.Add(new Vector3(-r.ReadSingle(), r.ReadSingle(), r.ReadSingle()));
                    for (int i = 0; i < nv; i++) nrm.Add(new Vector3(-r.ReadSingle(), r.ReadSingle(), r.ReadSingle()));
                    for (int i = 0; i < nv; i++) { float u = r.ReadSingle(), v = r.ReadSingle(); uv.Add(new Vector2(u, 1f - v)); }
                    var t = new int[ni];
                    for (int i = 0; i < ni; i++) t[i] = r.ReadInt32();
                    for (int i = 0; i + 2 < ni; i += 3) { int x = t[i + 1]; t[i + 1] = t[i + 2]; t[i + 2] = x; }
                    tris.Add(t);
                }
                var mesh = new Mesh { name = name, indexFormat = IndexFormat.UInt32 };
                mesh.SetVertices(pos);
                mesh.SetNormals(nrm);
                mesh.SetUVs(0, uv);
                mesh.subMeshCount = parts;
                for (int p = 0; p < parts; p++) mesh.SetTriangles(tris[p], p, false, bases[p]);
                mesh.RecalculateBounds();
                mesh.RecalculateTangents();
                mesh.UploadMeshData(false);
                return mesh;
            }
        }

        /// URP with Forward+, 4x MSAA, HDR, linear color, the manifest's shadow settings, no
        /// vertical sync; frame timing stats on for GPU time.
        static void SetUpProject(Manifest m)
        {
            PlayerSettings.colorSpace = ColorSpace.Linear;
            PlayerSettings.enableFrameTimingStats = true;
            PlayerSettings.fullScreenMode = FullScreenMode.Windowed;
            PlayerSettings.defaultScreenWidth = 1920;
            PlayerSettings.defaultScreenHeight = 1080;
            PlayerSettings.resizableWindow = false;
            PlayerSettings.runInBackground = true;
            Directory.CreateDirectory(Root);
            var data = ScriptableObject.CreateInstance<UniversalRendererData>();
            AssetDatabase.CreateAsset(data, Root + "/BenchRenderer.asset");
            var so = new SerializedObject(data);
            var mode = so.FindProperty("m_RenderingMode");
            if (mode != null) mode.intValue = 2; // Forward+
            so.ApplyModifiedPropertiesWithoutUndo();
            var asset = UniversalRenderPipelineAsset.Create(data);
            AssetDatabase.CreateAsset(asset, Root + "/BenchPipeline.asset");
            var a = new SerializedObject(asset);
            Set(a, "m_MSAA", 4);
            Set(a, "m_SupportsHDR", true);
            Set(a, "m_ShadowDistance", m.sun.shadow_far);
            Set(a, "m_ShadowCascadeCount", m.sun.shadow_cascades);
            Set(a, "m_MainLightShadowmapResolution", m.sun.shadow_resolution);
            Set(a, "m_MainLightShadowsSupported", true);
            Set(a, "m_SoftShadowsSupported", true);
            a.ApplyModifiedPropertiesWithoutUndo();
            GraphicsSettings.defaultRenderPipeline = asset;
            int level = QualitySettings.GetQualityLevel();
            for (int q = 0; q < QualitySettings.names.Length; q++)
            {
                QualitySettings.SetQualityLevel(q, false);
                QualitySettings.renderPipeline = asset;
                QualitySettings.vSyncCount = 0;
                QualitySettings.anisotropicFiltering = AnisotropicFiltering.Enable;
            }
            QualitySettings.SetQualityLevel(level, false);
        }

        static void Set(SerializedObject o, string name, int v) { var p = o.FindProperty(name); if (p != null) p.intValue = v; }
        static void Set(SerializedObject o, string name, float v) { var p = o.FindProperty(name); if (p != null) p.floatValue = v; }
        static void Set(SerializedObject o, string name, bool v) { var p = o.FindProperty(name); if (p != null) p.boolValue = v; }
    }
}
