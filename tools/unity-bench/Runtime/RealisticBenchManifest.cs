// The manifest written by Marionette's `perf_realistic_scene` operation (decision 0284), in
// the shape JsonUtility reads. Axes are glTF's (right-handed, +Y up); `RealisticBenchAxes`
// converts to Unity's left-handed frame by mirroring X.
using System;
using UnityEngine;

namespace MarionetteBench
{
    [Serializable] public class Textures { public string color, normal, metal_roughness, metal_smoothness; }
    [Serializable] public class MaterialInfo { public string name; public float[] tint; public float metallic, roughness; }
    [Serializable] public class ModelInfo
    {
        public string name, mesh, glb;
        public long triangles;
        public bool moving;
        public Textures textures;
        public MaterialInfo[] materials;
    }
    [Serializable] public class Instance { public int model; public float[] position, rotation; public float scale; }
    [Serializable] public class Mover
    {
        public int model, anchor;
        public float[] position, velocity, rotation;
        public float gain, scale;
    }
    [Serializable] public class Anchor { public float[] position; }
    [Serializable] public class CameraInfo { public float[] position, target, clear_color; public float fov_y_degrees, near, far; }
    [Serializable] public class SunInfo
    {
        public float[] direction, color;
        public float intensity, shadow_far;
        public int shadow_cascades, shadow_resolution;
    }
    [Serializable] public class AmbientInfo { public float[] sky, ground; }

    [Serializable]
    public class Manifest
    {
        public string format;
        public int version, texture_size, ground;
        public float dt;
        public long triangles;
        public CameraInfo camera;
        public SunInfo sun;
        public AmbientInfo ambient;
        public ModelInfo[] models;
        public Instance[] instances;
        public Anchor[] anchors;
        public Mover[] movers;
    }

    /// glTF (right-handed) to Unity (left-handed): mirror X.
    public static class RealisticBenchAxes
    {
        public static Vector3 Point(float[] p) => new Vector3(-p[0], p[1], p[2]);
        public static Quaternion Rotation(float[] q) => new Quaternion(q[0], -q[1], -q[2], q[3]);
    }
}
